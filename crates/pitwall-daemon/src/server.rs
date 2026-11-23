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
use pitwall_core::extensions::activity::Via;

/// How long a client may take to say hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// What this server offers (`welcome.caps`; plus `settings` with a
/// settings backend, `spaces` with a workspace backend, `viewer` with a
/// viewer backend and `events` with an event hub).
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
    /// Where `viewer.open` shows files (`None`: not offered).
    pub viewer: Option<Arc<dyn crate::ViewerBackend>>,
    /// The engine's event sink, when events go out over the socket
    /// (`None`: `events.subscribe` is not offered).
    pub events: Option<Arc<Hub>>,
    /// The extensions that are on, as callers (`None`: a registry of its
    /// own, empty).
    pub extensions: Option<Arc<crate::extensions::Registry>>,
}

/// What every connection shares.
pub struct Server {
    pub engine: Shared,
    pub approvals: Arc<Approvals>,
    pub identify: Arc<dyn Identify>,
    pub settings: Option<Arc<dyn crate::SettingsBackend>>,
    pub workspace: Option<Arc<dyn crate::WorkspaceBackend>>,
    pub viewer: Option<Arc<dyn crate::ViewerBackend>>,
    pub events: Option<Arc<Hub>>,
    pub extensions: Arc<crate::extensions::Registry>,
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
    let socket = cfg.socket.clone();
    listen(build(engine, approvals, identify, cfg), socket)
}

/// The server without a socket: what [`listen`] serves, and what the app
/// answers its extensions' calls with ([`Server::call_as`]).
pub fn build(engine: Shared, approvals: Arc<Approvals>, identify: Arc<dyn Identify>, cfg: Config) -> Arc<Server> {
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
    if cfg.viewer.is_some() {
        offered.push(caps::VIEWER);
        offered.push(caps::MCP);
        offered.push(caps::WEB);
        offered.push(caps::SHOW_SETTINGS);
    }
    Arc::new(Server {
        engine,
        approvals,
        identify,
        settings: cfg.settings,
        workspace: cfg.workspace,
        viewer: cfg.viewer,
        events: cfg.events,
        extensions: cfg.extensions.unwrap_or_default(),
        caps: offered,
        instance: uuid::Uuid::new_v4().to_string(),
        version: cfg.version,
    })
}

/// Listen on `socket` for `server`.
pub fn listen(server: Arc<Server>, socket: PathBuf) -> io::Result<Handle> {
    let listener = platform::bind(&socket)?;
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
    Ok(Handle { path: socket, stop, thread: Some(thread) })
}

impl Server {
    /// Extension `id`'s caller (its adapter's stdio).
    pub fn extension_caller(&self, id: &str) -> Option<Caller> {
        let name = self.extensions.of_id(id)?;
        Some(Caller { extension: Some((id.to_string(), name)), ..Caller::outside() })
    }

    /// Call `method` as `caller` (`via`: how it came; counted and logged
    /// for an extension). `events.subscribe` isn't one: see
    /// [`Server::subscribe_as`].
    pub fn call_as(&self, caller: &Caller, method: &str, params: serde_json::Value, via: Via) -> Result<serde_json::Value, ErrorBody> {
        if method == method::EVENTS_SUBSCRIBE {
            return Err(ErrorBody::new(code::UNSUPPORTED, "events.subscribe is answered by subscribe_as"));
        }
        methods::call(self, caller, method, params, via)
    }

    /// Subscribe `caller` to `req`'s topics: a queue of its own, the
    /// state now first, and the topics it gets. An extension's events
    /// stop when its permission is revoked ([`Server::may_get`]).
    pub fn subscribe_as(&self, caller: &Caller, req: &pitwall_proto::EventsSubscribe, via: Via) -> Result<(Arc<Queue>, Vec<&'static str>), ErrorBody> {
        let res = (|| {
            let Some(hub) = &self.events else {
                return Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall doesn't send events"));
            };
            let topics = self.topics_for(caller, req)?;
            let q = hub.subscribe(topics.clone());
            events::snapshot(&self.engine, &q);
            Ok((q, topics))
        })();
        if let Some((id, _)) = &caller.extension {
            let logged = res.as_ref().map(|(_, t)| serde_json::json!(t)).map_err(Clone::clone);
            self.extensions.record_call(id, via, method::EVENTS_SUBSCRIBE, &logged);
        }
        res
    }

    /// The topics `caller` gets for `req`: an extension needs its
    /// permission for each, and counts the call.
    fn topics_for(&self, caller: &Caller, req: &pitwall_proto::EventsSubscribe) -> Result<Vec<&'static str>, ErrorBody> {
        if let Some(ext) = &caller.extension {
            self.extensions.admit(&ext.0)?;
            methods::ext_allowed(method::EVENTS_SUBSCRIBE, ext, |p| self.extensions.allows(&ext.0, p))?;
            let topics = events::topics(caller, req)?;
            for t in &topics {
                let need = pitwall_core::extensions::permissions::of_topic(t).unwrap_or("");
                if !self.extensions.allows(&ext.0, need) {
                    return Err(ErrorBody::new(code::DENIED, format!("Not allowed: {} isn't granted {need} (for {t}).", ext.1)));
                }
            }
            return Ok(topics);
        }
        events::topics(caller, req)
    }

    /// Whether `caller` may still get event `name` (an extension's
    /// permission can be revoked while it is subscribed).
    pub fn may_get(&self, caller: &Caller, name: &str) -> bool {
        may_get(&self.extensions, caller.extension.as_ref(), name)
    }
}

/// Whether extension `ext` (none: not an extension) may get event `name`
/// now.
fn may_get(reg: &crate::extensions::Registry, ext: Option<&(String, String)>, name: &str) -> bool {
    let Some((id, _)) = ext else { return true };
    events::topic_of_event(name).and_then(pitwall_core::extensions::permissions::of_topic).is_some_and(|p| reg.allows(id, p))
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
        let (reg, ext) = (server.extensions.clone(), caller.extension.clone());
        sub.start(&writer, move |ev| may_get(&reg, ext.as_ref(), ev));
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
    /// Write its events out; `may`: whether the caller may still get one
    /// (an extension's grant is checked again for each).
    fn start(&mut self, writer: &Writer, may: impl Fn(&str) -> bool + Send + 'static) {
        let Some(q) = self.queue.clone().filter(|_| self.pusher.is_none()) else { return };
        let writer = writer.clone();
        self.pusher = std::thread::Builder::new()
            .name("pitwall-events".into())
            .spawn(move || {
                while let Some(ev) = q.pop() {
                    if !may(&ev.event) {
                        continue;
                    }
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
        if server.events.is_none() {
            return Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall doesn't send events"));
        }
        let p: pitwall_proto::EventsSubscribe = methods::params(req.params)?;
        let topics = match &sub.queue {
            Some(q) => {
                let topics = server.topics_for(caller, &p)?;
                q.set_topics(topics.clone());
                topics
            }
            None => {
                let (q, topics) = server.subscribe_as(caller, &p, Via::Socket)?;
                sub.queue = Some(q);
                topics
            }
        };
        Ok(EventsSubscribed { topics: topics.iter().map(|t| t.to_string()).collect() })
    })();
    match res.and_then(|v| serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))) {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, e),
    }
}

fn respond(server: &Server, caller: &Caller, req: Request) -> Response {
    match methods::call(server, caller, &req.method, req.params, Via::Socket) {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, e),
    }
}
