//! The viewer's side of `term.watch`: a peer agent's terminal, live and
//! view-only, on a connection of its own (so a busy terminal never delays
//! the link's events, and closing the pane closes it).
//!
//! A [`TermWatch`] dials the peer as its link does (the peer's current
//! address from Tailscale's status, both keys proven, never pairing),
//! asks for the agent by the peer's own id, and hands what comes to a
//! [`WatchSink`]: a reset with the source's size and the snapshot, live
//! output, size changes. When the connection drops it says so and dials
//! again with the link's backoff; the peer starts the new stream with a
//! fresh snapshot. It stops for good when the stream ends (the program
//! exited, the agent stopped or went away) or the peer refuses it.
//!
//! It never sends terminal data: there is no input path here.

use std::io::BufReader;
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use pitwall_proto::frame::{self, Frame};
use pitwall_proto::peer::{
    TermEnd, TermEnded, TermScreen, TermWatch as WatchParams, TermWatching, IDENTITY_CHANGED, NOT_PAIRED, PEER_MAX_FRAME, TERM_ENDED, TERM_RESET, TERM_RESIZED,
    TERM_WATCH,
};
use pitwall_proto::{code, ErrorBody, ServerMsg};

use crate::dial;
use crate::link::Pinger;
use crate::store::Paired;
use crate::{lock, Ctx};

/// What a watched terminal does, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// (Re)connected: start over at the source's size; the snapshot (the
    /// screen and scrollback so far) follows as output.
    Reset { cols: u16, rows: u16 },
    /// Terminal bytes, as the source's program wrote them.
    Output(Vec<u8>),
    /// The source's terminal is this size now.
    Resized { cols: u16, rows: u16 },
    /// The connection dropped: dialing again (why, in words).
    Reconnecting(String),
    /// Nothing more comes (why, in words): the program exited, the agent
    /// stopped, the peer refused.
    Ended(String),
}

/// Where a watch's events go. Called on the watch's own thread, in order;
/// never block for long.
pub trait WatchSink: Send + Sync {
    fn event(&self, ev: WatchEvent);
}

/// A running watch; dropping it closes the stream.
pub struct TermWatch {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    ctx: Arc<Ctx>,
    peer: Paired,
    port: u16,
    /// The peer's own id for the agent.
    agent: String,
    sink: Arc<dyn WatchSink>,
    ctl: Mutex<Ctl>,
    wake: Condvar,
}

#[derive(Default)]
struct Ctl {
    stop: bool,
    stream: Option<TcpStream>,
}

/// How one connection ended.
enum Outcome {
    /// Try again (the connection dropped, or the peer can't be reached).
    Lost(String),
    /// For good.
    Ended(String),
}

fn words(end: TermEnd) -> &'static str {
    match end {
        TermEnd::Exited => "it exited",
        TermEnd::Stopped => "it stopped",
        TermEnd::Gone => "it's no longer there",
        TermEnd::Unwatched => "closed",
    }
}

impl TermWatch {
    pub(crate) fn start(ctx: Arc<Ctx>, peer: Paired, port: u16, agent: String, sink: Arc<dyn WatchSink>) -> TermWatch {
        let shared = Arc::new(Shared { ctx, peer, port, agent, sink, ctl: Mutex::new(Ctl::default()), wake: Condvar::new() });
        let run = shared.clone();
        let thread = std::thread::Builder::new().name("pitwall-peer-term".into()).spawn(move || run.run()).ok();
        TermWatch { shared, thread }
    }
}

impl Drop for TermWatch {
    fn drop(&mut self) {
        {
            let mut ctl = lock(&self.shared.ctl);
            ctl.stop = true;
            if let Some(s) = &ctl.stream {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
        self.shared.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl TermWatch {
    /// A handle to end it from elsewhere ([`Ender::end`]).
    pub(crate) fn ender(&self) -> Ender {
        Ender(Arc::downgrade(&self.shared))
    }
}

/// Ends a watch that may already be gone (unpaired, showing turned off).
pub(crate) struct Ender(std::sync::Weak<Shared>);

impl Ender {
    pub fn alive(&self) -> bool {
        self.0.strong_count() > 0
    }

    pub fn peer(&self) -> Option<String> {
        self.0.upgrade().map(|s| s.peer.id.clone())
    }

    /// Stop it and tell its sink why.
    pub fn end(&self, why: &str) {
        let Some(s) = self.0.upgrade() else { return };
        {
            let mut ctl = lock(&s.ctl);
            if ctl.stop {
                return;
            }
            ctl.stop = true;
            if let Some(st) = &ctl.stream {
                let _ = st.shutdown(Shutdown::Both);
            }
        }
        s.wake.notify_all();
        s.sink.event(WatchEvent::Ended(why.into()));
    }
}

impl Shared {
    fn stopped(&self) -> bool {
        lock(&self.ctl).stop
    }

    fn wait(&self, d: std::time::Duration) {
        let deadline = Instant::now() + d;
        let mut ctl = lock(&self.ctl);
        while !ctl.stop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            ctl = self.wake.wait_timeout(ctl, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn run(&self) {
        let timing = &self.ctx.timing;
        let mut failures = 0usize;
        while !self.stopped() {
            let outcome = self.once(&mut failures);
            if self.stopped() {
                break;
            }
            match outcome {
                Outcome::Ended(why) => {
                    self.sink.event(WatchEvent::Ended(why));
                    break;
                }
                Outcome::Lost(why) => {
                    self.sink.event(WatchEvent::Reconnecting(why));
                    let d = timing.backoff.get(failures).or(timing.backoff.last()).copied().unwrap_or_default();
                    failures += 1;
                    self.wait(d);
                }
            }
        }
    }

    /// One connection: find, dial, prove, watch, follow until it ends.
    fn once(&self, failures: &mut usize) -> Outcome {
        let ctx = &self.ctx;
        let label = &self.peer.label;
        let lost = |why: &str| Outcome::Lost(format!("{label}: {why}"));
        let Ok(status) = ctx.tailnet.status() else { return lost("Tailscale isn't running") };
        let Ok(me) = status.me() else { return lost("Tailscale isn't running") };
        let Some(node) = status.node(&self.peer.stable_id) else { return lost("not on the tailnet") };
        if node.tagged() || node.user_id != me.user_id || status.login_of(node) != self.peer.login {
            return Outcome::Ended(format!("{label} now belongs to {}; pair again", status.login_of(node)));
        }
        let mut d = match dial::dial(ctx, node, self.port) {
            Ok(d) => d,
            Err(e) => return Outcome::Lost(e.message),
        };
        match d.auth(&ctx.key, &me.label(), false, Some(&self.peer.public_key)) {
            Ok(_) => {}
            Err(e) if e.code == IDENTITY_CHANGED || e.code == NOT_PAIRED => return Outcome::Ended(e.message),
            Err(e) => return Outcome::Lost(e.message),
        }
        {
            let mut ctl = lock(&self.ctl);
            if ctl.stop {
                return lost("closed");
            }
            ctl.stream = d.stream.try_clone().ok();
        }
        let _ = d.stream.set_read_timeout(Some(ctx.timing.dead_after));
        let out = match d.send(2, TERM_WATCH, WatchParams { agent_id: self.agent.clone() }) {
            Ok(()) => {
                let pinger = Pinger::start(&d.stream, ctx.timing.ping);
                let out = self.follow(&mut d.reader, failures);
                drop(pinger);
                out
            }
            Err(e) => lost(&e.to_string()),
        };
        lock(&self.ctl).stream = None;
        out
    }

    /// Read the stream into the sink until it ends.
    fn follow(&self, reader: &mut BufReader<TcpStream>, failures: &mut usize) -> Outcome {
        let label = &self.peer.label;
        let mut stream: Option<u32> = None;
        loop {
            let f = match frame::read_max(reader, PEER_MAX_FRAME) {
                Ok(Some(f)) => f,
                Ok(None) => return Outcome::Lost(format!("{label} closed the connection")),
                Err(e) => return Outcome::Lost(format!("{label}: {e}")),
            };
            match f {
                Frame::Term { stream: s, data } if Some(s) == stream => self.sink.event(WatchEvent::Output(data)),
                Frame::Term { .. } | Frame::Unknown(_) => {}
                Frame::Json(body) => match serde_json::from_slice::<ServerMsg>(&body) {
                    Ok(ServerMsg::Response(r)) if r.id == 2 => match r.into_result() {
                        Ok(v) => match serde_json::from_value::<TermWatching>(v) {
                            Ok(w) => {
                                stream = Some(w.stream);
                                *failures = 0;
                            }
                            Err(e) => return Outcome::Ended(format!("{label}: {e}")),
                        },
                        Err(e) => return Outcome::Ended(refused(label, e)),
                    },
                    Ok(ServerMsg::Response(r)) => {
                        if r.error.as_ref().is_some_and(|e| e.code == NOT_PAIRED) {
                            return Outcome::Ended(format!("{label} removed this machine; pair again"));
                        }
                    }
                    Ok(ServerMsg::Event(ev)) => {
                        let mine = |s: u32| Some(s) == stream;
                        match ev.event.as_str() {
                            TERM_RESET => {
                                if let Some(t) = serde_json::from_value::<TermScreen>(ev.data).ok().filter(|t| mine(t.stream)) {
                                    self.sink.event(WatchEvent::Reset { cols: t.cols, rows: t.rows });
                                }
                            }
                            TERM_RESIZED => {
                                if let Some(t) = serde_json::from_value::<TermScreen>(ev.data).ok().filter(|t| mine(t.stream)) {
                                    self.sink.event(WatchEvent::Resized { cols: t.cols, rows: t.rows });
                                }
                            }
                            TERM_ENDED => {
                                if let Some(t) = serde_json::from_value::<TermEnded>(ev.data).ok().filter(|t| mine(t.stream)) {
                                    return Outcome::Ended(words(t.reason).to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(e) => return Outcome::Lost(format!("{label}: {e}")),
                },
            }
        }
    }
}

/// `term.watch` refused, in words.
fn refused(label: &str, e: ErrorBody) -> String {
    match e.code.as_str() {
        code::NOT_RUNNING => "it isn't running".into(),
        code::NOT_FOUND => format!("it's no longer on {label}"),
        _ => e.message,
    }
}
