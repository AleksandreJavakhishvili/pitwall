//! Agention in Pitwall, as a tile (README.md): the threads your agents
//! open in your Agention server and the questions in them, read and
//! answered through Agention's operator API with `net`. The server is the
//! `server` setting's address, which the `net` permission names as
//! `{setting:server}`; the API key is the `apiKey` secret, held in memory
//! and sent only in the `Authorization` header, never shown or logged.
//!
//! - The list: threads grouped by agent and session (Pitwall's agent's
//!   name when the thread is from one of Pitwall's agents), those waiting
//!   on you first; a thread open beside it (or over it, when narrow) with
//!   its messages, its asks to answer (a suggestion, your words, or
//!   Decline), a reply box, Mark read and Archive.
//! - The same open asks in Pitwall's floating control (`asks`), a count on
//!   the sidebar row of each agent that waits on you (`badges`), and on
//!   the toolbar button.
//! - News arrives through the operator API's wait (a long poll), with a
//!   full read every minute; a failure is retried after 1, 2, 5, 10, 30
//!   and then every 60 seconds.

use crate::{api, model};

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

use api::{AskState, Author, Message, Route, Thread, ThreadRef};
use model::{Group, PitwallAgent};
use pitwall_extension::prelude::*;
use serde_json::Value;

/// The tile's id in extension.json.
const TILE: &str = "inbox";
/// From this width (logical pixels) the list and the thread show side by side.
const SPLIT_AT: f32 = 640.0;
/// The height of the header bar, as Pitwall's own tiles' bars.
const BAR: f32 = 36.0;
/// Requests at once (Pitwall allows 8; one is the wait).
const AT_ONCE: usize = 6;
/// How long the server holds a wait, and how long the tile gives it.
const WAIT_SECONDS: u32 = 25;
const WAIT_TIMEOUT: Duration = Duration::from_secs(40);
const TIMEOUT: Duration = Duration::from_secs(20);
/// A full read this often while connected.
const REFRESH_EVERY: Duration = Duration::from_secs(60);
/// Retries after a failure, then every minute.
const BACKOFF: [u64; 5] = [1, 2, 5, 10, 30];
/// The floating control shows at most this many questions, and these
/// many suggestions each.
const CONTROL_ASKS: usize = 50;
const CONTROL_SUGGESTIONS: usize = 8;

/// How the tile stands with the server.
#[derive(Debug, Clone, PartialEq, Default)]
enum Conn {
    /// A setting is missing or wrong: what to do.
    Setup(String),
    #[default]
    Connecting,
    Ready,
    /// The key is refused or lacks a scope.
    SignedOut(String),
    /// Not reachable, or it failed; retried.
    Down(String),
}

/// Why a request got no answer it could use.
#[derive(Debug, Clone)]
enum Fail {
    /// No answer: offline, timed out, refused by Pitwall.
    Net(String),
    /// Any other status, with the error shape's code and message.
    Status(u16, Option<(String, String)>),
    /// An answer it can't read.
    Bad(String),
}

/// What a failure means for the person.
struct Said {
    signed_out: bool,
    stale: bool,
    gone: bool,
    words: String,
}

fn said(route: &Route, f: &Fail) -> Said {
    let plain = |w: String| Said { signed_out: false, stale: false, gone: false, words: w };
    match f {
        Fail::Net(why) => plain(format!("Can't reach the Agention server ({why}).")),
        Fail::Bad(why) => plain(format!("The Agention server sent an answer this tile couldn't read ({why}).")),
        Fail::Status(401, _) => Said { signed_out: true, words: "Agention refused the API key: it is mistyped, revoked or expired. Create a new one in Agention under Settings → API keys.".into(), ..plain(String::new()) },
        Fail::Status(403, _) => Said { signed_out: true, words: missing_scope(route.scope.unwrap_or("needed")), ..plain(String::new()) },
        // A key without an operation's scope is answered not found, as if
        // the operation didn't exist; only a thread's routes can be gone.
        Fail::Status(404, _) if !route.thread => Said { signed_out: true, words: missing_scope(route.scope.unwrap_or("needed")), ..plain(String::new()) },
        Fail::Status(404, _) => Said { gone: true, ..plain("That thread isn't on the Agention server any more.".into()) },
        Fail::Status(409, Some((code, _))) if code == "stale" => Said { stale: true, ..plain("The agent wrote something new in this thread; read it, then send again.".into()) },
        Fail::Status(429, _) => plain("Agention asked to slow down; trying again soon.".into()),
        Fail::Status(s @ (400 | 409 | 413), e) => plain(match e {
            Some((_, m)) if !m.is_empty() => format!("Agention refused it: {m}"),
            _ => format!("Agention refused it (status {s})."),
        }),
        Fail::Status(s, _) => plain(format!("The Agention server had a problem (status {s}).")),
    }
}

fn missing_scope(scope: &str) -> String {
    format!("The API key doesn't have the {scope} scope. Create a key with the threads:write and sessions:read scopes in Agention under Settings → API keys.")
}

type Done = Box<dyn FnOnce(&mut Agention, Result<Value, Fail>, &mut Cx<Agention>)>;

struct Queued {
    route: Route,
    timeout: Duration,
    gen: u64,
    done: Done,
}

/// A thread's messages as last read.
#[derive(Debug, Clone, Default)]
struct Read {
    last_seq: u64,
    /// Messages before this sequence weren't read.
    after: u64,
    messages: Vec<Message>,
}

/// One view's own state.
#[derive(Debug, Clone, Default)]
struct ViewState {
    open: Option<String>,
    /// All threads, not only those waiting on you.
    all: bool,
    width: f32,
}

/// What the person is writing in an ask (by ask id) or a reply (by thread).
#[derive(Debug, Clone, Default)]
struct AskDraft {
    picks: BTreeSet<u32>,
    text: String,
}

#[derive(Default)]
struct Agention {
    gen: u64,
    conn: Conn,
    base: String,
    /// The API key, in memory only.
    key: Option<String>,
    threads: Vec<Thread>,
    /// Sessions' status titles, by agent and session.
    titles: BTreeMap<(String, String), String>,
    reads: BTreeMap<String, Read>,
    agents: Vec<PitwallAgent>,
    views: BTreeMap<String, ViewState>,
    queue: VecDeque<Queued>,
    in_flight: usize,
    waiting: bool,
    since: Option<String>,
    failures: usize,
    retry: Option<Timer>,
    every: Option<Timer>,
    soon: Option<Timer>,
    refreshing: bool,
    again: bool,
    replies: BTreeMap<String, String>,
    asking: BTreeMap<String, AskDraft>,
    /// Threads or asks being sent.
    sending: BTreeSet<String>,
    /// What went wrong with a thread's last action, by thread.
    notice: BTreeMap<String, String>,
    /// Details opened, by thread and message.
    details: BTreeSet<(String, u64)>,
    /// What was last given to the floating control, the sidebar and the
    /// toolbar, so a set is sent only when it changes.
    shown_asks: Option<Vec<asks::Ask>>,
    shown_badges: Option<Vec<badges::Badge>>,
    shown_toolbar: Option<(u32, bool)>,
    service: Option<service::Info>,
    counter: u64,
}

impl Tile for Agention {
    fn new(cx: &mut Cx<Self>) -> Self {
        let mut this = Agention { service: Some(service::info()), ..Agention::default() };
        this.read_agents();
        this.connect(cx);
        this
    }

    fn on_view_opened(&mut self, view: &ViewInfo, _cx: &mut Cx<Self>) {
        self.views.entry(view.id.clone()).or_default();
    }

    fn on_view_closed(&mut self, view: &str, _cx: &mut Cx<Self>) {
        self.views.remove(view);
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            Event::SettingsChanged => self.connect(cx),
            Event::GrantsChanged(_) => {
                self.read_agents();
                if matches!(self.conn, Conn::Setup(_) | Conn::Down(_)) {
                    self.connect(cx);
                }
                self.publish();
                cx.notify();
            }
            Event::AgentsChanged => {
                self.read_agents();
                self.publish();
                cx.notify();
            }
            Event::AskAnswered(a) => self.answered_in_control(a.clone(), cx),
            Event::ServiceChanged(info) => {
                let up = info.state == service::State::Up;
                self.service = Some(info.clone());
                if up && matches!(self.conn, Conn::Down(_)) {
                    self.connect(cx);
                }
                cx.notify();
            }
            Event::Resize { view, width, .. } => {
                let vs = self.views.entry(view.clone()).or_default();
                let was = vs.width >= SPLIT_AT;
                vs.width = *width;
                if was != (*width >= SPLIT_AT) {
                    cx.notify();
                }
            }
            Event::Theme(_) => cx.notify(),
            _ => {}
        }
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        let vs = self.views.get(view).cloned().unwrap_or_default();
        let root = v_flex().size_full().bg(Token::Surface).text_color(Token::Text).child(self.head(view, &vs, cx));
        let problem = self.problem(cx);
        if self.threads.is_empty() || matches!(self.conn, Conn::Setup(_)) {
            return root.child(problem.unwrap_or_else(|| self.nothing_yet(cx)));
        }
        let root = match problem {
            Some(p) => root.child(div().id("problem").flex_none().p_2().child(p)),
            None => root,
        };
        let split = vs.width >= SPLIT_AT;
        let open = vs.open.clone().filter(|id| self.thread(id).is_some());
        let body = if split {
            div().flex_row().id("main")
                .flex_1()
                .min_h(px(0.))
                .child(self.list(view, &vs, cx).w(px(300.)).flex_none().border_r_1().border_color(Token::Line))
                .child(match &open {
                    Some(id) => self.detail(view, id, false, cx),
                    None => v_flex().flex_1().child(empty_state("No thread open").icon(Icon::Inbox).text("Pick a thread to read it and answer its questions.")),
                })
        } else {
            match &open {
                Some(id) => div().flex_row().id("main").flex_1().min_h(px(0.)).child(self.detail(view, id, true, cx)),
                None => div().flex_row().id("main").flex_1().min_h(px(0.)).child(self.list(view, &vs, cx).flex_1()),
            }
        };
        root.child(body)
    }
}

// ---- Talking to the server -------------------------------------------------

impl Agention {
    fn request_id(&mut self) -> String {
        use std::hash::{BuildHasher, Hasher};
        self.counter += 1;
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(self.counter);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        format!("pitwall-{:016x}{:04x}", h.finish(), self.counter & 0xffff)
    }

    /// Send `route` when there is room; `done` hears the JSON answer.
    fn call(&mut self, cx: &mut Cx<Self>, route: Route, timeout: Duration, done: impl FnOnce(&mut Agention, Result<Value, Fail>, &mut Cx<Agention>) + 'static) {
        self.queue.push_back(Queued { route, timeout, gen: self.gen, done: Box::new(done) });
        self.pump(cx);
    }

    fn pump(&mut self, cx: &mut Cx<Self>) {
        while self.in_flight < AT_ONCE {
            let Some(q) = self.queue.pop_front() else { return };
            if q.gen != self.gen {
                continue;
            }
            let mut req = net::Request::new(q.route.method, format!("{}{}", self.base, q.route.path)).header("Accept", "application/json").timeout(q.timeout);
            if q.route.scope.is_some() {
                if let Some(k) = &self.key {
                    req = req.header("Authorization", format!("Bearer {k}"));
                }
            }
            if let Some(b) = &q.route.body {
                req = req.json(b);
            }
            let (r2, gen) = (q.route, q.gen);
            // Kept outside the callback, so a request refused at once
            // still hears why.
            let slot = std::rc::Rc::new(std::cell::RefCell::new(Some(q.done)));
            let theirs = slot.clone();
            let sent = cx.fetch(req, move |this: &mut Agention, answer, cx| {
                this.in_flight = this.in_flight.saturating_sub(1);
                let done = theirs.borrow_mut().take();
                if let (Some(done), true) = (done, gen == this.gen) {
                    let got = match answer {
                        Err(e) => Err(Fail::Net(api::redact(&net_words(&e)))),
                        Ok(r) if r.ok() => serde_json::from_slice::<Value>(&r.body).map_err(|_| Fail::Bad("not JSON".into())),
                        Ok(r) => Err(Fail::Status(r.status, serde_json::from_slice::<Value>(&r.body).ok().as_ref().and_then(api::error_of))),
                    };
                    if let Err(f) = &got {
                        log::debug(format!("{} {}: {}", r2.method, r2.path.split('?').next().unwrap_or(""), said(&r2, f).words));
                    }
                    done(this, got, cx);
                }
                this.pump(cx);
            });
            match sent {
                Ok(_) => self.in_flight += 1,
                Err(e) => {
                    let done = slot.borrow_mut().take();
                    if let Some(done) = done {
                        done(self, Err(Fail::Net(net_words(&e))), cx);
                    }
                }
            }
        }
    }

    /// Start again from the settings: check them, then the server's API
    /// version and the key's scopes, then read everything and wait.
    fn connect(&mut self, cx: &mut Cx<Self>) {
        self.gen += 1;
        self.queue.clear();
        self.waiting = false;
        self.since = None;
        for t in [self.retry.take(), self.every.take(), self.soon.take()].into_iter().flatten() {
            cx.cancel(t);
        }
        self.key = None;
        let base = match api::base_url(&settings::text("server").unwrap_or_default()) {
            Ok(b) => b,
            Err(e) => return self.set_conn(Conn::Setup(e), cx),
        };
        self.base = base;
        let key = match settings::secret("apiKey") {
            Ok(Some(k)) if !k.trim().is_empty() => k.trim().to_string(),
            Ok(_) => return self.set_conn(Conn::Setup("Paste your Agention API key in Settings: create one in Agention under Settings → API keys, with the threads:write and sessions:read scopes.".into()), cx),
            Err(e) => return self.set_conn(Conn::Setup(format!("The API key can't be read: {e}")), cx),
        };
        if !api::is_key(&key) {
            return self.set_conn(Conn::Setup("That doesn't look like an Agention API key: a key starts with agention_ followed by 43 letters, digits, - and _.".into()), cx);
        }
        self.key = Some(key);
        if !cx.is_granted("net") {
            return self.set_conn(Conn::Setup("Allow Agention to reach your server: grant its network permission in Settings.".into()), cx);
        }
        self.set_conn(Conn::Connecting, cx);
        let route = api::versions();
        self.call(cx, route.clone(), TIMEOUT, move |this, got, cx| {
            let versions = match got {
                Ok(v) => api::version_list(&v),
                Err(Fail::Status(404, _)) => Err("This Agention server has no operator API: update Agention to v0.7.0 or later.".into()),
                // Down or busy: tried again, as any other failure.
                Err(f @ Fail::Status(429 | 500.., _)) => return this.failed(&route, &f, cx),
                Err(Fail::Bad(_)) | Err(Fail::Status(..)) => Err("That address doesn't answer like an Agention server. Check Server address: it is the one you open Agention at in your browser.".into()),
                Err(f) => return this.failed(&route, &f, cx),
            };
            match versions {
                Ok(v) if v.contains(&api::API_VERSION) => this.check_scopes(cx),
                Ok(v) => this.set_conn(Conn::Setup(format!("This Agention server serves operator API version {}, and this tile speaks version 1: update the extension.", v.iter().map(u64::to_string).collect::<Vec<_>>().join(", "))), cx),
                Err(why) => this.set_conn(Conn::Setup(why), cx),
            }
        });
    }

    fn check_scopes(&mut self, cx: &mut Cx<Self>) {
        let probe = api::probe_threads_write();
        self.call(cx, probe.clone(), TIMEOUT, move |this, got, cx| {
            match got {
                // Refused for its empty body: the scope is there.
                Err(Fail::Status(400, _)) | Ok(_) => this.read_sessions(None, Vec::new(), true, cx),
                Err(f) => this.failed(&probe, &f, cx),
            }
        });
    }

    /// The sessions' titles, page by page; on connecting, then the threads.
    fn read_sessions(&mut self, cursor: Option<String>, mut got: Vec<api::Session>, connecting: bool, cx: &mut Cx<Self>) {
        let route = api::sessions(cursor.as_deref());
        self.call(cx, route.clone(), TIMEOUT, move |this, answer, cx| {
            let page = match answer {
                Ok(v) => api::session_page(&v).map_err(Fail::Bad),
                Err(f) => Err(f),
            };
            match page {
                Ok((sessions, next)) => {
                    got.extend(sessions);
                    match next {
                        Some(n) if got.len() < 5000 => this.read_sessions(Some(n), got, connecting, cx),
                        _ => {
                            this.titles = got.into_iter().filter(|s| !s.title.is_empty()).map(|s| ((s.agent, s.session), s.title)).collect();
                            if connecting {
                                this.refresh(cx);
                            } else {
                                cx.notify();
                            }
                        }
                    }
                }
                Err(f) => this.failed(&route, &f, cx),
            }
        });
    }

    /// Read the threads again (one read at a time; once more after it when
    /// asked meanwhile), then the threads it needs the messages of.
    fn refresh(&mut self, cx: &mut Cx<Self>) {
        if self.refreshing {
            self.again = true;
            return;
        }
        self.refreshing = true;
        self.again = false;
        self.read_page(None, Vec::new(), cx);
    }

    fn read_page(&mut self, cursor: Option<String>, mut got: Vec<Thread>, cx: &mut Cx<Self>) {
        let route = api::list_threads(cursor.as_deref());
        self.call(cx, route.clone(), TIMEOUT, move |this, answer, cx| {
            let page = match answer {
                Ok(v) => api::thread_page(&v).map_err(Fail::Bad),
                Err(f) => Err(f),
            };
            match page {
                Ok((threads, next)) => {
                    let empty = threads.is_empty();
                    for t in threads {
                        if got.len() < api::limits::THREADS && !got.iter().any(|g| g.at == t.at) {
                            got.push(t);
                        }
                    }
                    match next {
                        Some(n) if !empty && got.len() < api::limits::THREADS => this.read_page(Some(n), got, cx),
                        _ => this.listed(got, cx),
                    }
                }
                Err(f) => {
                    this.refreshing = false;
                    this.failed(&route, &f, cx);
                }
            }
        });
    }

    fn listed(&mut self, threads: Vec<Thread>, cx: &mut Cx<Self>) {
        self.refreshing = false;
        self.threads = threads;
        let ids: BTreeSet<String> = self.threads.iter().map(Thread::id).collect();
        self.reads.retain(|id, _| ids.contains(id));
        let open: BTreeSet<String> = self.views.values().filter_map(|v| v.open.clone()).collect();
        let need: Vec<String> = self.threads.iter().filter(|t| (t.open_asks > 0 || open.contains(&t.id())) && self.reads.get(&t.id()).is_none_or(|r| r.last_seq != t.last_seq)).map(Thread::id).collect();
        for id in need {
            self.read_thread(&id, cx);
        }
        if self.conn != Conn::Ready {
            self.failures = 0;
            self.set_conn(Conn::Ready, cx);
            let t = cx.every(REFRESH_EVERY, |this: &mut Agention, cx| {
                this.refresh(cx);
                this.read_sessions(None, Vec::new(), false, cx);
            });
            self.every = t;
        }
        self.wait(cx);
        self.publish();
        cx.notify();
        if self.again {
            self.refresh(cx);
        }
    }

    /// Read thread `id`'s messages: the latest 100, and back to its oldest
    /// open ask (at most 200 in all).
    fn read_thread(&mut self, id: &str, cx: &mut Cx<Self>) {
        let Some(t) = self.thread(id).cloned() else { return };
        let mut after = t.last_seq.saturating_sub(100);
        if let Some(first) = t.open_ask_seqs.first() {
            after = after.min(first.saturating_sub(1));
        }
        after = after.max(t.last_seq.saturating_sub(api::limits::MESSAGES as u64));
        self.read_from(t.at, after, after, Vec::new(), cx);
    }

    fn read_from(&mut self, at: ThreadRef, start: u64, after: u64, mut got: Vec<Message>, cx: &mut Cx<Self>) {
        let route = api::read_thread(&at, after, api::limits::PAGE);
        self.call(cx, route.clone(), TIMEOUT, move |this, answer, cx| {
            let read = match answer {
                Ok(v) => api::thread_read(&v).map_err(Fail::Bad),
                Err(f) => Err(f),
            };
            let id = at.id();
            match read {
                Ok(r) => {
                    got.extend(r.messages);
                    let more = r.page_count == api::limits::PAGE && r.page_end > after && r.page_end < r.thread.last_seq && got.len() < api::limits::MESSAGES;
                    if more {
                        return this.read_from(at, start, r.page_end, got, cx);
                    }
                    if let Some(t) = this.threads.iter_mut().find(|t| t.at == at) {
                        *t = r.thread.clone();
                    }
                    this.reads.insert(id, Read { last_seq: r.thread.last_seq, after: start, messages: got });
                    this.publish();
                    cx.notify();
                }
                Err(f) => {
                    let s = said(&route, &f);
                    if s.gone {
                        this.threads.retain(|t| t.at != at);
                        this.reads.remove(&id);
                        this.publish();
                        cx.notify();
                    } else if s.signed_out {
                        this.failed(&route, &f, cx);
                    } else {
                        this.notice.insert(id, s.words);
                        cx.notify();
                    }
                }
            }
        });
    }

    /// Wait for news (one wait at a time, while connected).
    fn wait(&mut self, cx: &mut Cx<Self>) {
        if self.waiting || self.conn != Conn::Ready {
            return;
        }
        self.waiting = true;
        let route = api::wait(self.since.as_deref(), WAIT_SECONDS);
        self.call(cx, route.clone(), WAIT_TIMEOUT, move |this, answer, cx| {
            this.waiting = false;
            let news = match answer {
                Ok(v) => api::waited(&v).map_err(Fail::Bad),
                Err(f) => Err(f),
            };
            match news {
                Ok((threads, since)) => {
                    if since.is_some() {
                        this.since = since;
                    }
                    if threads.is_empty() {
                        this.wait(cx);
                    } else {
                        // Read everything again; the wait goes on after it.
                        this.refresh(cx);
                    }
                }
                Err(Fail::Net(why)) if why.contains("no answer in time") => this.wait(cx),
                Err(f) => this.failed(&route, &f, cx),
            }
        });
    }

    /// A request failed: signed out, or not reachable (tried again later).
    fn failed(&mut self, route: &Route, f: &Fail, cx: &mut Cx<Self>) {
        let s = said(route, f);
        self.gen += 1;
        self.queue.clear();
        self.waiting = false;
        self.refreshing = false;
        for t in [self.every.take(), self.soon.take(), self.retry.take()].into_iter().flatten() {
            cx.cancel(t);
        }
        if s.signed_out {
            return self.set_conn(Conn::SignedOut(s.words), cx);
        }
        let secs = BACKOFF.get(self.failures).copied().unwrap_or(60);
        self.failures += 1;
        self.set_conn(Conn::Down(format!("{} Trying again in {secs} s.", s.words)), cx);
        self.retry = cx.after(Duration::from_secs(secs), |this: &mut Agention, cx| {
            this.retry = None;
            let failures = this.failures;
            this.connect(cx);
            this.failures = failures;
        });
    }

    fn set_conn(&mut self, c: Conn, cx: &mut Cx<Self>) {
        if !matches!(c, Conn::Ready | Conn::Connecting) && !matches!(c, Conn::Down(_)) {
            // Not set up or signed out: nothing shown is current.
            self.threads.clear();
            self.reads.clear();
        }
        self.conn = c;
        self.publish();
        cx.notify();
    }

    fn read_agents(&mut self) {
        self.agents = agents::list().map(|l| l.into_iter().map(|a| PitwallAgent { id: a.id, name: a.name, project: a.project }).collect()).unwrap_or_default();
    }

    fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id() == id)
    }

    /// The latest sequence of thread `id` the person was shown.
    fn seen_through(&self, id: &str) -> u64 {
        self.reads.get(id).map(|r| r.last_seq).or_else(|| self.thread(id).map(|t| t.last_seq)).unwrap_or(0)
    }

    /// The floating control's questions, the sidebar's badges and the
    /// toolbar's count, from what is read now; each sent when it changed.
    fn publish(&mut self) {
        let ready = self.conn == Conn::Ready || matches!(self.conn, Conn::Down(_));
        let msgs: BTreeMap<String, Vec<Message>> = if ready { self.reads.iter().map(|(k, r)| (k.clone(), r.messages.clone())).collect() } else { BTreeMap::new() };
        let threads: &[Thread] = if ready { &self.threads } else { &[] };
        let open = model::open_asks(threads, &msgs, &self.agents, CONTROL_ASKS);
        let list: Vec<asks::Ask> = open
            .iter()
            .map(|o| {
                let mut a = asks::Ask::new(o.id.clone(), cut(&o.ask.question, 2000)).title(cut(&o.title, 200));
                if let Some(f) = &o.from {
                    a = a.from_agent(f.clone());
                }
                for s in o.ask.suggestions.iter().take(CONTROL_SUGGESTIONS) {
                    a = if s.description.is_empty() { a.suggest(cut(&s.label, 80)) } else { a.suggest_with(cut(&s.label, 80), cut(&s.description, 200)) };
                }
                if o.ask.multiple {
                    a = a.multiple();
                }
                if o.ask.blocking {
                    a = a.blocking();
                }
                a
            })
            .collect();
        if self.shown_asks.as_ref() != Some(&list) {
            if let Err(e) = asks::set(&list) {
                log::warn(format!("the questions couldn't be shown in the floating control: {e}"));
            }
            self.shown_asks = Some(list);
        }
        let marks: Vec<badges::Badge> = model::waiting_by_agent(threads, &self.agents)
            .into_iter()
            .map(|(id, (n, blocking))| {
                let words = if n == 1 { "1 question waits for you".to_string() } else { format!("{n} questions wait for you") };
                badges::Badge::count(badges::Row::Agent(id), n).tone(if blocking { Tone::Bad } else { Tone::Warn }).tooltip(words).open(TILE)
            })
            .collect();
        if self.shown_badges.as_ref() != Some(&marks) {
            if let Err(e) = badges::set(&marks) {
                log::warn(format!("the sidebar badges couldn't be set: {e}"));
            }
            self.shown_badges = Some(marks);
        }
        let total: u32 = threads.iter().filter(|t| !t.archived).map(|t| t.open_asks).sum();
        let blocking = threads.iter().any(|t| t.blocking && t.open_asks > 0);
        if self.shown_toolbar != Some((total, blocking)) {
            toolbar::set_badge(if total > 0 { toolbar::Badge::Count(total) } else { toolbar::Badge::None }, if blocking { toolbar::Tone::Attention } else { toolbar::Tone::Normal });
            toolbar::set_tooltip(&match total {
                0 => "Agention".to_string(),
                1 => "Agention: 1 question waits for you".to_string(),
                n => format!("Agention: {n} questions wait for you"),
            });
            self.shown_toolbar = Some((total, blocking));
        }
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max - 1).collect();
        o.push('…');
        o
    }
}

fn net_words(e: &net::Error) -> String {
    match e {
        net::Error::Denied(host) => format!("Pitwall may not reach {host}: check Server address and the network permission"),
        net::Error::TimedOut => "no answer in time".into(),
        net::Error::Failed(why) => why.chars().take(200).collect(),
        other => other.to_string(),
    }
}

// ---- What the person does ------------------------------------------------------

impl Agention {
    fn open_thread(&mut self, view: &str, id: &str, cx: &mut Cx<Self>) {
        self.views.entry(view.to_string()).or_default().open = Some(id.to_string());
        self.notice.remove(id);
        self.read_thread(id, cx);
        cx.notify();
    }

    fn close_thread(&mut self, view: &str, cx: &mut Cx<Self>) {
        if let Some(v) = self.views.get_mut(view) {
            v.open = None;
        }
        cx.notify();
    }

    /// Send `route` for thread `id` as the person's action `what` (busy
    /// meanwhile); afterwards read it and the list again.
    fn act(&mut self, id: &str, busy: String, route: Route, cx: &mut Cx<Self>, then: impl FnOnce(&mut Agention, &mut Cx<Agention>) + 'static) {
        if self.sending.contains(&busy) {
            return;
        }
        self.sending.insert(busy.clone());
        self.notice.remove(id);
        let tid = id.to_string();
        let r2 = route.clone();
        self.call(cx, route, TIMEOUT, move |this, got, cx| {
            this.sending.remove(&busy);
            match got {
                Ok(_) => then(this, cx),
                Err(f) => {
                    let s = said(&r2, &f);
                    if s.signed_out {
                        return this.failed(&r2, &f, cx);
                    }
                    this.notice.insert(tid.clone(), s.words);
                    if s.stale {
                        this.read_thread(&tid, cx);
                    }
                }
            }
            this.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }

    fn mark_read(&mut self, id: &str, cx: &mut Cx<Self>) {
        let Some(at) = ThreadRef::parse(id) else { return };
        let route = api::mark_read(&at, self.seen_through(id));
        self.act(id, format!("{id}@read"), route, cx, |_, _| {});
    }

    fn archive(&mut self, id: &str, cx: &mut Cx<Self>) {
        let Some(at) = ThreadRef::parse(id) else { return };
        let route = api::archive(&at, self.seen_through(id));
        let tid = id.to_string();
        self.act(id, format!("{id}@archive"), route, cx, move |this, _| {
            for v in this.views.values_mut().filter(|v| v.open.as_deref() == Some(&tid)) {
                v.open = None;
            }
            ui::toast("Archived: it comes back when the agent writes again", Tone::Good);
        });
    }

    fn send_reply(&mut self, id: &str, cx: &mut Cx<Self>) {
        let Some(at) = ThreadRef::parse(id) else { return };
        let words = self.replies.get(id).cloned().unwrap_or_default();
        let rid = self.request_id();
        match api::message_body(&rid, self.seen_through(id), &words) {
            Ok(body) => {
                let tid = id.to_string();
                self.act(id, format!("{id}@reply"), api::send(&at, body), cx, move |this, cx| {
                    this.replies.remove(&tid);
                    this.read_thread(&tid, cx);
                });
            }
            Err(e) => {
                self.notice.insert(id.to_string(), format!("Not sent: {e}."));
                cx.notify();
            }
        }
    }

    /// Answer ask `number` of thread `id` from the tile.
    fn answer(&mut self, id: &str, number: u64, reply: api::Reply, cx: &mut Cx<Self>) {
        let Some(at) = ThreadRef::parse(id) else { return };
        let rid = self.request_id();
        let ask = model::ask_id(id, number);
        match api::answers_body(&rid, self.seen_through(id), &[reply]) {
            Ok(body) => {
                let (tid, a2) = (id.to_string(), ask.clone());
                self.act(id, ask, api::send(&at, body), cx, move |this, cx| {
                    this.asking.remove(&a2);
                    this.read_thread(&tid, cx);
                });
            }
            Err(e) => {
                self.notice.insert(id.to_string(), format!("Not sent: {e}."));
                cx.notify();
            }
        }
    }

    /// The person answered in the floating control.
    fn answered_in_control(&mut self, a: asks::Answer, cx: &mut Cx<Self>) {
        let refuse = |why: &str| asks::reply(&a.id, Some(why));
        let Some((id, number)) = model::parse_ask_id(&a.id) else { return refuse("That question isn't one this tile knows.") };
        let (Some(at), true) = (ThreadRef::parse(&id), self.conn == Conn::Ready) else { return refuse("Not connected to Agention right now; try again in a moment.") };
        let reply = api::Reply { ask: number, picks: a.picks.clone(), text: a.text.clone().unwrap_or_default(), declined: a.declined };
        let rid = self.request_id();
        let body = match api::answers_body(&rid, self.seen_through(&id), &[reply]) {
            Ok(b) => b,
            Err(e) => return refuse(&e),
        };
        let route = api::send(&at, body);
        let r2 = route.clone();
        let (aid, tid) = (a.id.clone(), id.clone());
        self.call(cx, route, TIMEOUT, move |this, got, cx| {
            match got {
                Ok(_) => {
                    asks::reply(&aid, None);
                    this.read_thread(&tid, cx);
                }
                Err(f) => {
                    let s = said(&r2, &f);
                    asks::reply(&aid, Some(&s.words));
                    if s.stale {
                        this.read_thread(&tid, cx);
                    }
                }
            }
            this.refresh(cx);
        });
    }
}

// ---- Drawing -------------------------------------------------------------------

fn bar() -> Div {
    h_flex().id("head").flex_none().h(px(BAR)).px_2().gap_2().items_center().border_b_1().border_color(Token::Line)
}

impl Agention {
    fn status_words(&self) -> (&'static str, Tone) {
        match &self.conn {
            Conn::Ready => ("Connected", Tone::Good),
            Conn::Connecting => ("Connecting…", Tone::Info),
            Conn::Setup(_) => ("Needs setup", Tone::Warn),
            Conn::SignedOut(_) => ("Signed out", Tone::Bad),
            Conn::Down(_) => ("Not connected", Tone::Warn),
        }
    }

    fn head(&self, view: &str, vs: &ViewState, cx: &mut Cx<Self>) -> Div {
        let (words, tone) = self.status_words();
        let v = view.to_string();
        let mut b = bar().child(icon(Icon::Inbox).text_color(Token::Text2)).child(text("Agention").text_sm().font_semibold()).child(badge(words).id("status").tone(tone)).child(div().flex_1());
        if !self.threads.is_empty() {
            b = b.child(
                segmented()
                    .id("filter")
                    .option("waiting", "Waiting on you")
                    .option("all", "All")
                    .selected(if vs.all { "all" } else { "waiting" })
                    .on_change(cx.listener(move |this: &mut Agention, key: &str, cx| {
                        this.views.entry(v.clone()).or_default().all = key == "all";
                        cx.notify();
                    })),
            );
        }
        b.child(icon_button(Icon::Refresh, "Refresh").id("refresh").tooltip("Read again from Agention").on_click(cx.listener(|this: &mut Agention, _, cx| {
            if this.conn == Conn::Ready {
                this.refresh(cx);
            } else {
                this.failures = 0;
                this.connect(cx);
            }
        })))
    }

    /// What stands in the way, as a banner or an empty state.
    fn problem(&self, cx: &mut Cx<Self>) -> Option<AnyElement> {
        let mut open_settings = || cx.listener(|_: &mut Agention, _, _| settings::open());
        Some(match &self.conn {
            Conn::Setup(why) => v_flex().id("setup").flex_1().child(empty_state("Set up Agention").icon(Icon::Inbox).text(why.clone()).action("Open Settings", open_settings())).into_element(),
            Conn::SignedOut(why) => banner(Tone::Bad, why.clone()).id("signed-out").title("Signed out").action("Open Settings", open_settings()).into_element(),
            Conn::Down(why) => {
                let mut words = why.clone();
                let svc = self.service.as_ref().filter(|s| !matches!(s.state, service::State::Up | service::State::None | service::State::Starting));
                if let Some(s) = svc {
                    words = format!("{words} Agention's server on this computer isn't running{}.", s.why.as_ref().map(|w| format!(": {w}")).unwrap_or_default());
                }
                let b = banner(Tone::Warn, words).id("down").title("Not connected");
                if svc.is_some() {
                    b.action("Open its card", cx.listener(|_: &mut Agention, _, _| service::open_card())).into_element()
                } else {
                    b.action("Try again", cx.listener(|this: &mut Agention, _, cx| {
                        this.failures = 0;
                        this.connect(cx);
                    }))
                    .into_element()
                }
            }
            Conn::Connecting | Conn::Ready => return None,
        })
    }

    fn nothing_yet(&self, _cx: &mut Cx<Self>) -> AnyElement {
        match self.conn {
            Conn::Connecting => v_flex().id("connecting").flex_1().items_center().justify_center().gap_2().child(spinner()).child(text("Connecting to Agention…").text_sm().text_color(Token::Text2)).into_element(),
            _ => v_flex().id("empty").flex_1().child(empty_state("No threads yet").icon(Icon::Inbox).text("When an agent asks you something through Agention, it shows here.")).into_element(),
        }
    }

    fn list(&self, view: &str, vs: &ViewState, cx: &mut Cx<Self>) -> Div {
        let groups = model::groups(&self.threads, &self.agents, &self.titles, !vs.all);
        let mut rows = v_flex().id("rows").flex_1().min_h(px(0.)).overflow_y_scroll().py_1();
        if groups.is_empty() {
            let v = view.to_string();
            rows = rows.child(empty_state("Nothing waits on you").icon(Icon::Check).text("Your agents' questions show here.").action("Show all threads", cx.listener(move |this: &mut Agention, _, cx| {
                this.views.entry(v.clone()).or_default().all = true;
                cx.notify();
            })));
        }
        for g in &groups {
            rows = rows.child(self.group_head(g));
            for id in &g.threads {
                if let Some(t) = self.thread(id) {
                    rows = rows.child(self.row(view, vs, t, cx));
                }
            }
        }
        v_flex().id("list").min_w(px(0.)).min_h(px(0.)).child(rows)
    }

    fn group_head(&self, g: &Group) -> Div {
        let key = format!("g-{}~{}", g.agent, g.session);
        let mut h = h_flex().id(key).px_3().pt_2().pb_1().gap_2().items_center().child(avatar(g.label.clone()).small()).child(text(g.label.clone()).text_xs().font_semibold().text_color(Token::Text2).truncate());
        if g.pitwall.is_none() {
            h = h.tooltip("Not one of the agents Pitwall started");
        }
        let mut col = v_flex().child(h);
        if !g.status.is_empty() {
            col = col.child(text(g.status.clone()).text_xs().text_color(Token::Text3).px_3().pb_1().truncate());
        }
        col
    }

    fn row(&self, view: &str, vs: &ViewState, t: &Thread, cx: &mut Cx<Self>) -> AnyElement {
        let id = t.id();
        let selected = vs.open.as_deref() == Some(id.as_str());
        let mut meta = h_flex().gap_1p5().items_center().child(relative_time_ms(t.updated_ms).text_xs().text_color(Token::Text3));
        if t.open_asks > 0 {
            let words = if t.open_asks == 1 { "1 question".to_string() } else { format!("{} questions", t.open_asks) };
            meta = meta.child(badge(words).tone(if t.blocking { Tone::Bad } else { Tone::Warn }));
        }
        if t.unread > 0 {
            meta = meta.child(badge(format!("{} new", t.unread)).tone(Tone::Info));
        }
        let (v, tid) = (view.to_string(), id.clone());
        let mut row = v_flex()
            .id(format!("t-{}", model::key_of(&id)))
            .mx_2()
            .px_2()
            .py_2()
            .gap_0p5()
            .rounded_md()
            .cursor_pointer()
            .focusable()
            .child(text(if t.title.is_empty() { "(no title)".to_string() } else { t.title.clone() }).text_sm().font_medium().truncate())
            .child(meta)
            .on_click(cx.listener(move |this: &mut Agention, _, cx| this.open_thread(&v, &tid, cx)));
        if selected {
            row = row.bg(Token::Surface2);
        }
        row.into_element()
    }

    fn detail(&self, view: &str, id: &str, narrow: bool, cx: &mut Cx<Self>) -> Div {
        let Some(t) = self.thread(id).cloned() else { return v_flex() };
        let group = model::matching(&self.agents, &t.at.agent, &t.at.session).map_or_else(|| format!("{} · {}", t.at.agent, t.at.session), |a| a.name.clone());
        let mut head = bar();
        if narrow {
            let v = view.to_string();
            head = head.child(icon_button(Icon::Back, "Back").id("back").on_click(cx.listener(move |this: &mut Agention, _, cx| this.close_thread(&v, cx))));
        }
        head = head.child(v_flex().flex_1().min_w(px(0.)).child(text(t.title.clone()).text_sm().font_semibold().truncate()).child(text(group).text_xs().text_color(Token::Text3).truncate()));
        if t.unread > 0 {
            let tid = id.to_string();
            head = head.child(button("Mark read").small().id("read").disabled(self.sending.contains(&format!("{id}@read"))).on_click(cx.listener(move |this: &mut Agention, _, cx| this.mark_read(&tid, cx))));
        }
        let tid = id.to_string();
        head = head.child(button("Archive").small().icon(Icon::Check).id("archive").tooltip("Archive: it comes back when the agent writes again").disabled(self.sending.contains(&format!("{id}@archive"))).on_click(cx.listener(move |this: &mut Agention, _, cx| this.archive(&tid, cx))));

        let mut body = v_flex().id("messages").flex_1().min_h(px(0.)).overflow_y_scroll().p_3().gap_3();
        if let Some(n) = self.notice.get(id) {
            body = body.child(banner(Tone::Warn, n.clone()).id("notice"));
        }
        match self.reads.get(id) {
            None => body = body.child(h_flex().gap_2().items_center().child(spinner()).child(text("Reading…").text_sm().text_color(Token::Text2))),
            Some(r) => {
                if r.after > 0 {
                    body = body.child(text("Earlier messages are in Agention.").id("earlier").text_xs().text_color(Token::Text3));
                }
                for m in &r.messages {
                    body = body.child(self.message(id, m, cx));
                }
            }
        }
        let reply_key = id.to_string();
        let send_key = id.to_string();
        let words = self.replies.get(id).cloned().unwrap_or_default();
        let busy = self.sending.contains(&format!("{id}@reply"));
        let reply = v_flex()
            .id("reply-box")
            .flex_none()
            .p_2()
            .gap_1p5()
            .border_t_1()
            .border_color(Token::Line)
            .child(
                input("reply")
                    .multiline()
                    .rows(3)
                    .placeholder("Write to the agent")
                    .value(words.clone())
                    .on_change(cx.listener(move |this: &mut Agention, v: &str, cx| {
                        let was_empty = this.replies.get(&reply_key).is_none_or(|s| s.trim().is_empty());
                        this.replies.insert(reply_key.clone(), v.to_string());
                        if was_empty != v.trim().is_empty() {
                            cx.notify();
                        }
                    })),
            )
            .child(h_flex().id("actions").justify_end().child(button("Send").primary().small().icon(Icon::Send).id("send-reply").disabled(busy || words.trim().is_empty()).on_click(cx.listener(move |this: &mut Agention, _, cx| this.send_reply(&send_key, cx)))));
        v_flex().flex_1().min_w(px(0.)).min_h(px(0.)).id("thread").child(head).child(body).child(reply)
    }

    fn message(&self, thread: &str, m: &Message, cx: &mut Cx<Self>) -> Div {
        let who = match &m.author {
            Author::Agent { agent, session } => model::matching(&self.agents, agent, session).map_or_else(|| agent.clone(), |a| a.name.clone()),
            Author::Operator { name } => if name.is_empty() { "You".to_string() } else { format!("{name} (you)") },
        };
        let mine = matches!(m.author, Author::Operator { .. });
        let mut col = v_flex()
            .id(format!("m{}", m.seq))
            .gap_1()
            .child(h_flex().gap_2().items_center().child(avatar(who.clone()).small()).child(text(who).text_xs().font_semibold()).child(relative_time_ms(m.time_ms).text_xs().text_color(Token::Text3)));
        let mut inner = v_flex().id("body").gap_1p5().pl_3();
        if !m.text.is_empty() {
            inner = inner.child(markdown(m.text.clone()).text_sm().text_color(if mine { Token::Text2 } else { Token::Text }));
        }
        if !m.detail.is_empty() && m.detail.trim() != m.text.trim() {
            let key = (thread.to_string(), m.seq);
            let open = self.details.contains(&key);
            inner = inner.child(
                disclosure("More", open)
                    .id("more")
                    .on_toggle(cx.listener(move |this: &mut Agention, open: &bool, cx| {
                        if *open {
                            this.details.insert(key.clone());
                        } else {
                            this.details.remove(&key);
                        }
                        cx.notify();
                    }))
                    .child(markdown(m.detail.clone()).text_sm()),
            );
        }
        for a in &m.answers {
            let what = answer_words(&a.answer);
            inner = inner.child(text(format!("Answered “{}”: {what}", a.question)).text_xs().text_color(Token::Text3));
        }
        for a in &m.asks {
            inner = inner.child(self.ask(thread, a, cx));
        }
        col = col.child(inner);
        col
    }

    fn ask(&self, thread: &str, a: &api::Ask, cx: &mut Cx<Self>) -> Div {
        let aid = model::ask_id(thread, a.number);
        let mut card = v_flex().id(format!("a{}", a.number)).p_2().gap_1p5().rounded_md().border_1().border_color(Token::Line).bg(Token::Surface2);
        let mut top = h_flex().gap_2().items_center().child(text(a.question.clone()).text_sm().font_medium().flex_1());
        if a.blocking && a.state == AskState::Open {
            top = top.child(badge("Waiting on you").tone(Tone::Bad));
        }
        card = card.child(top);
        match &a.state {
            AskState::Open => {
                let draft = self.asking.get(&aid).cloned().unwrap_or_default();
                let busy = self.sending.contains(&aid);
                let mut picks = h_flex().id("picks").flex_wrap().gap_1p5();
                for (i, s) in a.suggestions.iter().enumerate().take(api::input::PICKS) {
                    let (t, aid2, n, multiple) = (thread.to_string(), aid.clone(), a.number, a.multiple);
                    let chosen = draft.picks.contains(&(i as u32));
                    let mut b = button(s.label.clone()).small().id(format!("s{i}")).disabled(busy);
                    if chosen {
                        b = b.primary();
                    }
                    if !s.description.is_empty() {
                        b = b.tooltip(s.description.clone());
                    }
                    picks = picks.child(b.on_click(cx.listener(move |this: &mut Agention, _, cx| {
                        if multiple {
                            let d = this.asking.entry(aid2.clone()).or_default();
                            if !d.picks.remove(&(i as u32)) {
                                d.picks.insert(i as u32);
                            }
                            cx.notify();
                        } else {
                            // One pick: sent at once, with any words typed.
                            let text = this.asking.get(&aid2).map(|d| d.text.clone()).unwrap_or_default();
                            this.answer(&t, n, api::Reply { ask: n, picks: vec![i as u32], text, declined: false }, cx);
                        }
                    })));
                }
                if !a.suggestions.is_empty() {
                    card = card.child(picks);
                }
                let aid3 = aid.clone();
                card = card.child(input("words").placeholder(if a.suggestions.is_empty() { "Your answer" } else { "Or say something else" }).value(draft.text.clone()).disabled(busy).on_change(cx.listener(move |this: &mut Agention, v: &str, cx| {
                    let d = this.asking.entry(aid3.clone()).or_default();
                    let was = d.text.trim().is_empty();
                    d.text = v.to_string();
                    if was != v.trim().is_empty() {
                        cx.notify();
                    }
                })));
                let ready = !draft.text.trim().is_empty() || (a.multiple && !draft.picks.is_empty());
                let (t1, t2, n) = (thread.to_string(), thread.to_string(), a.number);
                let picks_now: Vec<u32> = if a.multiple { draft.picks.iter().copied().collect() } else { Vec::new() };
                let words = draft.text.clone();
                card = card.child(
                    h_flex().id("actions")
                        .gap_2()
                        .justify_end()
                        .child(button("Decline").small().link().id("decline").disabled(busy).tooltip("Tell the agent you won't answer").on_click(cx.listener(move |this: &mut Agention, _, cx| this.answer(&t1, n, api::Reply { ask: n, declined: true, ..Default::default() }, cx))))
                        .child(button("Send").small().primary().id("send").disabled(busy || !ready).on_click(cx.listener(move |this: &mut Agention, _, cx| this.answer(&t2, n, api::Reply { ask: n, picks: picks_now.clone(), text: words.clone(), declined: false }, cx)))),
                );
            }
            AskState::Answered(ans) => card = card.child(text(format!("You answered: {}", answer_words(ans))).text_xs().text_color(Token::Good)),
            AskState::Declined => card = card.child(text("You declined").text_xs().text_color(Token::Text3)),
            AskState::Withdrawn(why) => card = card.child(text(if why.is_empty() { "Withdrawn by the agent".to_string() } else { format!("Withdrawn: {why}") }).text_xs().text_color(Token::Text3)),
            AskState::Closed => card = card.child(text("Asked again later in the thread").text_xs().text_color(Token::Text3)),
        }
        card
    }
}

fn answer_words(a: &api::Answer) -> String {
    if a.declined {
        return "declined".into();
    }
    let mut parts: Vec<String> = a.labels.clone();
    if !a.text.is_empty() {
        parts.push(format!("“{}”", cut(&a.text, 300)));
    }
    if parts.is_empty() {
        "answered".into()
    } else {
        parts.join(", ")
    }
}

pitwall_extension::export!(Agention);
