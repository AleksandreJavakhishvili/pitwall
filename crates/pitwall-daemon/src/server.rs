//! The socket server: accept, identify the peer, handshake, then answer
//! requests one at a time per connection (a thread each; a request waiting
//! for the user's approval blocks only its own connection). A connection
//! that subscribed to events gets a second thread writing them out.

use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use pitwall_core::Shared;
use pitwall_proto::frame::{self, Frame};
use pitwall_proto::{answer_hello, caps, code, method, ClientHello, ErrorBody, EventsSubscribed, Range, Request, Response, ServerHello, ServerMsg};

use crate::approvals::Approvals;
use crate::events::{self, Hub, Queue};
use crate::identity::{Caller, Identify};
use crate::{methods, platform};

/// How long a client may take to say hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// What this server offers (`welcome.caps`; plus `settings` with a
/// settings backend, `spaces` with a workspace backend and `events` with an
/// event hub).
pub const CAPS: &[&str] = &[caps::AGENTS, caps::SESSIONS, caps::APPROVALS, caps::MANAGE];

pub struct Config {
    /// Where to listen (`~/.pitwall/run/pitwalld.sock`).
    pub socket: PathBuf,
    /// Shown to clients (`welcome.daemon`).
    pub version: String,
    /// Where `settings.*` read and apply settings (`None`: not offered).
    pub settings: Option<Arc<dyn crate::SettingsBackend>>,
    /// Where `space.*` and `agent.move` arrange spaces (`None`: not offered).
    pub workspace: Option<Arc<dyn crate::WorkspaceBackend>>,
    /// The engine's event sink, when events go out over the socket
    /// (`None`: `events.subscribe` is not offered).
    pub events: Option<Arc<Hub>>,
}

/// What every connection shares.
pub struct Server {
    pub engine: Shared,
    pub approvals: Arc<Approvals>,
    pub identify: Arc<dyn Identify>,
    pub settings: Option<Arc<dyn crate::SettingsBackend>>,
    pub workspace: Option<Arc<dyn crate::WorkspaceBackend>>,
    pub events: Option<Arc<Hub>>,
    caps: Vec<&'static str>,
    instance: String,
    version: String,
}

/// A running server; [`stop`](Handle::stop) ends it and removes the socket.
pub struct Handle {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Handle {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stop accepting connections and remove the socket. Connections that
    /// are open finish their current request.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        platform::poke(&self.path);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start listening. Requests go to `engine`; risky ones wait in `approvals`;
/// `identify` says who each connection is.
pub fn serve(engine: Shared, approvals: Arc<Approvals>, identify: Arc<dyn Identify>, cfg: Config) -> io::Result<Handle> {
    let listener = platform::bind(&cfg.socket)?;
    let mut offered = CAPS.to_vec();
    if cfg.settings.is_some() {
        offered.push(caps::SETTINGS);
    }
    if cfg.workspace.is_some() {
        offered.push(caps::SPACES);
    }
    if cfg.events.is_some() {
        offered.push(caps::EVENTS);
    }
    let server = Arc::new(Server {
        engine,
        approvals,
        identify,
        settings: cfg.settings,
        workspace: cfg.workspace,
        events: cfg.events,
        caps: offered,
        instance: uuid::Uuid::new_v4().to_string(),
        version: cfg.version,
    });
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new().name("pitwall-server".into()).spawn(move || {
        for conn in listener.incoming() {
            if flag.load(Ordering::SeqCst) {
                break;
            }
            let Ok(conn) = conn else { continue };
            let server = server.clone();
            let _ = std::thread::Builder::new().name("pitwall-conn".into()).spawn(move || {
                if let Err(e) = handle(&server, conn) {
                    if !matches!(e.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) {
                        eprintln!("pitwall: socket client: {e}");
                    }
                }
            });
        }
    })?;
    Ok(Handle { path: cfg.socket, stop, thread: Some(thread) })
}

fn handle(server: &Server, conn: platform::Stream) -> io::Result<()> {
    let caller = server.identify.identify(platform::peer_pid(&conn));
    conn.set_read_timeout(Some(HELLO_TIMEOUT))?;
    let mut reader = BufReader::new(conn.try_clone()?);
    let mut writer = BufWriter::new(conn.try_clone()?);
    let Some(Frame::Json(body)) = frame::read(&mut reader)? else { return Ok(()) };
    let hello = match serde_json::from_slice::<ClientHello>(&body) {
        Ok(ClientHello::Hello(h)) => h,
        Err(_) => {
            let reject = ServerHello::Reject(pitwall_proto::Reject {
                code: code::BAD_HELLO.into(),
                daemon: server.version.clone(),
                protocol: Range::ours(),
            });
            return frame::write_json(&mut writer, &reject);
        }
    };
    let answer = answer_hello(&hello, Range::ours(), &server.version, &server.caps, &server.instance);
    let welcomed = matches!(answer, ServerHello::Welcome(_));
    frame::write_json(&mut writer, &answer)?;
    if !welcomed {
        return Ok(());
    }
    conn.set_read_timeout(None)?;
    let writer = Arc::new(Mutex::new(writer));
    // Ends the event thread however the connection ends.
    let mut sub = Subscription::default();
    loop {
        let body = match frame::read(&mut reader)? {
            Some(Frame::Json(b)) => b,
            // No terminal streams are open yet; unknown frames are ignored.
            Some(_) => continue,
            None => return Ok(()),
        };
        let resp = match serde_json::from_slice::<Request>(&body) {
            Ok(req) if req.method == method::EVENTS_SUBSCRIBE => subscribe(server, &caller, &mut sub, req),
            Ok(req) => respond(server, &caller, req),
            Err(e) => Response::err(0, ErrorBody::new(code::BAD_PARAMS, format!("not a request: {e}"))),
        };
        frame::write_json(&mut *lock(&writer), &resp)?;
        // After the answer, so the subscriber sees it before any event.
        sub.start(&writer);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type Writer = Arc<Mutex<BufWriter<platform::Stream>>>;

/// A connection's events: its queue in the hub, and the thread writing it.
#[derive(Default)]
struct Subscription {
    queue: Option<Arc<Queue>>,
    pusher: Option<JoinHandle<()>>,
}

impl Subscription {
    fn start(&mut self, writer: &Writer) {
        let Some(q) = self.queue.clone().filter(|_| self.pusher.is_none()) else { return };
        let writer = writer.clone();
        self.pusher = std::thread::Builder::new()
            .name("pitwall-events".into())
            .spawn(move || {
                while let Some(ev) = q.pop() {
                    // A client that stopped reading blocks only this thread;
                    // the engine keeps filling (and trimming) the queue.
                    if frame::write_json(&mut *lock(&writer), &ServerMsg::Event(ev)).is_err() {
                        q.close();
                    }
                }
            })
            .ok();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(q) = &self.queue {
            q.close();
        }
        if let Some(t) = self.pusher.take() {
            let _ = t.join();
        }
    }
}

/// `events.subscribe`: per connection, so answered here rather than in
/// [`methods::dispatch`]. Subscribing again changes the topics.
fn subscribe(server: &Server, caller: &Caller, sub: &mut Subscription, req: Request) -> Response {
    let res = (|| {
        let Some(hub) = &server.events else {
            return Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall doesn't send events"));
        };
        let p: pitwall_proto::EventsSubscribe = methods::params(req.params)?;
        let topics = events::topics(caller, &p)?;
        match &sub.queue {
            Some(q) => q.set_topics(topics.clone()),
            None => {
                let q = hub.subscribe(topics.clone());
                events::snapshot(&server.engine, &q);
                sub.queue = Some(q);
            }
        }
        Ok(EventsSubscribed { topics: topics.iter().map(|t| t.to_string()).collect() })
    })();
    match res.and_then(|v| serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))) {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, e),
    }
}

fn respond(server: &Server, caller: &Caller, req: Request) -> Response {
    match methods::dispatch(server, caller, &req.method, req.params) {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, e),
    }
}
