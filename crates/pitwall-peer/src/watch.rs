//! The listener's side of `term.watch` (tailnet-peers.md "Terminal
//! streams"): one of this Pitwall's agents' terminals, view-only, to a
//! paired viewer.
//!
//! Each watch is a fan-out subscriber of the agent's `TermHost`, as a pane
//! here is ([`TermHost::watch_output`]: no redraw nudge, so a viewer never
//! resizes or otherwise touches the program). The subscriber only queues
//! bytes; a thread per watch writes them out as terminal frames of at most
//! [`TERM_CHUNK`] bytes. The queue is bounded: a viewer more than
//! [`Limits::watch_behind`] bytes behind is cut off the fan-out (the agent
//! never waits for it), and once the writer is through, the watch starts
//! over with `term.reset` and a fresh snapshot (the ring as it is now).
//!
//! The first `term.reset` carries the source's size; `term.resized` follows
//! its changes. A watch ends with `term.ended` when the program exits, the
//! agent stops or is no longer listed, or the viewer unwatches; it ends
//! without a word when the connection closes (revoked, unpaired, sharing
//! turned off). Start and end go to the audit log.
//!
//! Nothing here reads from the viewer: input is not part of View.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pitwall_core::term::TermHost;
use pitwall_proto::peer::{TermEnd, TermEnded, TermScreen, MAX_WATCHES, TERM_CHUNK, TERM_ENDED, TERM_RESET, TERM_RESIZED};
use pitwall_proto::{code, frame, ErrorBody, Event, ServerMsg};

use crate::server::Feed;
use crate::store::AuditEntry;
use crate::{lock, Ctx};

/// How often a watch looks at the size and, every [`CHECK`], whether the
/// agent is still there (output wakes it at once).
const TICK: Duration = Duration::from_millis(200);
const CHECK: Duration = Duration::from_secs(1);

pub(crate) type Writer = Arc<Mutex<std::io::BufWriter<std::net::TcpStream>>>;

/// Who is watching, for the audit log and the pairing check.
#[derive(Clone)]
pub(crate) struct Viewer {
    pub label: String,
    pub login: String,
    pub node: String,
    pub stable_id: String,
    pub public_key: String,
}

impl Viewer {
    fn audit(&self, event: &str, agent: &str, detail: Option<&str>) -> AuditEntry {
        let e = AuditEntry::new(event).peer(self.label.clone()).login(self.login.clone()).node(self.node.clone()).method(pitwall_proto::peer::TERM_WATCH);
        e.detail(match detail {
            Some(d) => format!("agent {agent}: {d}"),
            None => format!("agent {agent}"),
        })
    }
}

/// A connection's watches.
pub(crate) struct Watches {
    list: Vec<Watch>,
    /// Asked for, answered, not started yet (the answer goes first).
    pending: Vec<Watch>,
    next: u32,
}

impl Default for Watches {
    fn default() -> Watches {
        Watches { list: Vec::new(), pending: Vec::new(), next: 1 }
    }
}

struct Watch {
    stream: u32,
    agent: String,
    host: Option<Arc<TermHost>>,
    q: Arc<Queue>,
    thread: Option<JoinHandle<()>>,
}

/// What the subscriber hands the writer.
struct Queue {
    st: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    /// The subscription that may push (older ones are cut off).
    generation: u64,
    /// It fell behind: start over.
    behind: bool,
    /// Stop: with `term.ended` (`Some`), or quietly (the connection is gone).
    stop: Option<Option<TermEnd>>,
}

impl Queue {
    fn stop(&self, end: Option<TermEnd>) {
        let mut st = lock(&self.st);
        st.stop.get_or_insert(end);
        self.wake.notify_all();
    }
}

impl Watches {
    /// Watch `agent` (already checked: listed, with a terminal `host`).
    /// Returns the stream id; the stream starts with [`start_pending`]
    /// (after the answer is written).
    pub fn add(&mut self, agent: &str, host: Arc<TermHost>) -> Result<u32, ErrorBody> {
        self.list.retain(|w| w.thread.as_ref().is_some_and(|t| !t.is_finished()));
        if self.list.len() + self.pending.len() >= MAX_WATCHES {
            return Err(ErrorBody::new(code::CONFLICT, format!("at most {MAX_WATCHES} terminals per connection")));
        }
        let stream = self.next;
        self.next += 1;
        let q = Arc::new(Queue { st: Mutex::new(State::default()), wake: Condvar::new() });
        self.pending.push(Watch { stream, agent: agent.to_string(), host: Some(host), q, thread: None });
        Ok(stream)
    }

    /// Start what [`add`](Self::add) set up.
    pub fn start_pending(&mut self, ctx: &Arc<Ctx>, feed: &Arc<dyn Feed>, writer: &Writer, viewer: &Viewer) {
        for mut w in std::mem::take(&mut self.pending) {
            let run = Run {
                ctx: ctx.clone(),
                feed: feed.clone(),
                writer: writer.clone(),
                viewer: viewer.clone(),
                stream: w.stream,
                agent: w.agent.clone(),
                q: w.q.clone(),
            };
            let host = w.host.take().expect("pending");
            ctx.store.audit(viewer.audit("watch", &w.agent, None));
            w.thread = std::thread::Builder::new().name("pitwall-peer-watch".into()).spawn(move || run.run(host)).ok();
            self.list.push(w);
        }
    }

    /// `term.unwatch`: `term.ended {unwatched}` follows.
    pub fn remove(&mut self, stream: u32) -> Result<(), ErrorBody> {
        let Some(at) = self.list.iter().position(|w| w.stream == stream) else {
            return Err(ErrorBody::new(code::NOT_FOUND, format!("no stream {stream}")));
        };
        let w = self.list.remove(at);
        w.q.stop(Some(TermEnd::Unwatched));
        drop(w);
        Ok(())
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // The connection is gone (or it was unwatched, already stopping).
        self.q.stop(None);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One watch's writer thread.
struct Run {
    ctx: Arc<Ctx>,
    feed: Arc<dyn Feed>,
    writer: Writer,
    viewer: Viewer,
    stream: u32,
    agent: String,
    q: Arc<Queue>,
}

/// How one subscription ended.
enum Next {
    /// Start over with a fresh snapshot (fell behind, or a new terminal).
    Again(Arc<TermHost>),
    End(Option<TermEnd>),
}

impl Run {
    fn run(self, mut host: Arc<TermHost>) {
        let end = loop {
            match self.follow(&host) {
                Next::Again(h) => host = h,
                Next::End(end) => break end,
            }
        };
        if let Some(reason) = end {
            let _ = self.event(TERM_ENDED, TermEnded { stream: self.stream, reason });
        }
        let why = match end {
            Some(TermEnd::Exited) => "exited",
            Some(TermEnd::Stopped) => "stopped",
            Some(TermEnd::Gone) => "gone",
            Some(TermEnd::Unwatched) => "unwatched",
            None => "closed",
        };
        self.ctx.store.audit(self.viewer.audit("unwatch", &self.agent, Some(why)));
    }

    fn event(&self, name: &str, data: impl serde::Serialize) -> std::io::Result<()> {
        let ev = ServerMsg::Event(Event { event: name.into(), data: serde_json::to_value(data).unwrap_or_default() });
        let mut w = lock(&self.writer);
        w.write_all(&frame::json(&ev))?;
        w.flush()
    }

    fn bytes(&self, chunks: &[Vec<u8>]) -> std::io::Result<()> {
        let mut w = lock(&self.writer);
        for c in chunks {
            for piece in c.chunks(TERM_CHUNK) {
                w.write_all(&frame::term(self.stream, piece))?;
            }
        }
        w.flush()
    }

    /// One subscription: reset, the snapshot, live output, until it falls
    /// behind, the terminal changes or the watch ends.
    fn follow(&self, host: &Arc<TermHost>) -> Next {
        let (cols, rows) = host.size();
        if self.event(TERM_RESET, TermScreen { stream: self.stream, cols, rows }).is_err() {
            return Next::End(None);
        }
        let generation = {
            let mut st = lock(&self.q.st);
            if let Some(end) = st.stop {
                return Next::End(end);
            }
            st.generation += 1;
            st.chunks.clear();
            st.bytes = 0;
            st.behind = false;
            st.generation
        };
        let q = self.q.clone();
        let behind_max = self.ctx.limits.watch_behind;
        let mut first = true;
        // Never blocks: the agent's output goes on whatever the viewer does.
        let sub = host.watch_output(Box::new(move |b: &[u8]| {
            let mut st = lock(&q.st);
            if st.generation != generation || st.stop.is_some() || st.behind {
                return false;
            }
            // The snapshot always fits; live output may lag this far.
            if !first && st.bytes + b.len() > behind_max {
                st.behind = true;
                st.chunks.clear();
                st.bytes = 0;
                q.wake.notify_all();
                return false;
            }
            first = false;
            st.bytes += b.len();
            st.chunks.push_back(b.to_vec());
            q.wake.notify_all();
            true
        }));
        let next = self.pump(host, (cols, rows));
        host.detach(sub);
        next
    }

    fn pump(&self, host: &Arc<TermHost>, mut size: (u16, u16)) -> Next {
        let mut checked = Instant::now();
        loop {
            let (chunks, behind, stop) = {
                let mut st = lock(&self.q.st);
                if st.chunks.is_empty() && !st.behind && st.stop.is_none() {
                    st = self.q.wake.wait_timeout(st, TICK).unwrap_or_else(|e| e.into_inner()).0;
                }
                st.bytes = 0;
                (std::mem::take(&mut st.chunks), st.behind, st.stop)
            };
            if let Some(end) = stop {
                return Next::End(end);
            }
            if behind {
                return Next::Again(host.clone());
            }
            if !chunks.is_empty() {
                let chunks: Vec<Vec<u8>> = chunks.into();
                if self.bytes(&chunks).is_err() {
                    return Next::End(None);
                }
            }
            let now = host.size();
            if now != size {
                size = now;
                if self.event(TERM_RESIZED, TermScreen { stream: self.stream, cols: now.0, rows: now.1 }).is_err() {
                    return Next::End(None);
                }
            }
            let drained = lock(&self.q.st).chunks.is_empty();
            if host.ended() && host.eof_is_exit() && drained {
                return Next::End(Some(TermEnd::Exited));
            }
            if checked.elapsed() >= CHECK {
                checked = Instant::now();
                if let Some(next) = self.check(host) {
                    return next;
                }
            }
        }
    }

    /// Still paired, still listed, still the same terminal?
    fn check(&self, host: &Arc<TermHost>) -> Option<Next> {
        let v = &self.viewer;
        if !self.ctx.store.pairings().viewers.iter().any(|p| p.stable_id == v.stable_id && p.public_key == v.public_key) {
            return Some(Next::End(None));
        }
        if !self.feed.views().iter().any(|a| a.id == self.agent) {
            return Some(Next::End(Some(TermEnd::Gone)));
        }
        match self.feed.terminal(&self.agent) {
            None => Some(Next::End(Some(TermEnd::Stopped))),
            // Restarted, or re-attached: the new terminal from the top.
            Some(h) if !Arc::ptr_eq(&h, host) => Some(Next::Again(h)),
            Some(_) => None,
        }
    }
}
