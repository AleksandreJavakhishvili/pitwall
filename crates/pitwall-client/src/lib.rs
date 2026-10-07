//! A client of Pitwall's socket (architecture.md §4): connect, handshake,
//! typed calls. Blocking and single-threaded: one request at a time, which
//! is what the CLI needs. Events that arrive meanwhile are kept
//! ([`Client::take_events`]); terminal frames are skipped (no terminal
//! streams are opened yet).

mod platform;

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use pitwall_proto::frame::{self, Frame};
use pitwall_proto::{
    method, AgentCreate, AgentView, ApprovalAnswer, ApprovalView, ClientHello, CreateForm, ErrorBody, Event, FormRequest, Hello, ProviderMachines, Range,
    Reject, Request, Role, ScannedPlace, ServerHello, ServerMsg, SessionAdd, SessionAdded, SessionFilter, Welcome,
};

pub use pitwall_proto as proto;

#[derive(Debug)]
pub enum Error {
    /// Nothing listens there (Pitwall isn't running), or it can't be reached.
    Connect { path: PathBuf, source: std::io::Error },
    /// The server speaks no protocol version this client does.
    Rejected(Reject),
    Io(std::io::Error),
    /// The server sent something that isn't the protocol.
    Protocol(String),
    /// The server answered with an error.
    Server(ErrorBody),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Connect { path, source } => write!(f, "can't reach Pitwall at {} ({source}). Is Pitwall running?", path.display()),
            Error::Rejected(r) => write!(
                f,
                "Pitwall {} speaks protocol {}–{}, this client {}–{}",
                r.daemon,
                r.protocol.min,
                r.protocol.max,
                Range::ours().min,
                Range::ours().max
            ),
            Error::Io(e) => write!(f, "connection to Pitwall failed: {e}"),
            Error::Protocol(m) => write!(f, "unexpected answer from Pitwall: {m}"),
            Error::Server(e) => write!(f, "{}", e.message),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Where Pitwall listens: `$PITWALL_CLI_SOCKET` (set in every agent and
/// terminal Pitwall starts), else `~/.pitwall/run/pitwalld.sock`.
pub fn socket_path() -> PathBuf {
    match std::env::var_os(pitwall_proto::SOCKET_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => platform::data_dir().join("run").join(pitwall_proto::SOCKET_NAME),
    }
}

/// A connection to Pitwall's socket without the handshake (tests,
/// diagnostics): a Unix socket, or a named pipe on Windows.
pub type RawConn = platform::Conn;

pub fn connect_raw(path: &Path) -> std::io::Result<RawConn> {
    platform::connect(path)
}

pub struct Client {
    conn: platform::Conn,
    next_id: u64,
    welcome: Welcome,
    events: Vec<Event>,
}

impl Client {
    /// Connect as a CLI client.
    pub fn connect(path: &Path) -> Result<Client> {
        Client::connect_as(path, &format!("pitwall-client/{}", env!("CARGO_PKG_VERSION")), Role::Cli)
    }

    pub fn connect_as(path: &Path, client: &str, role: Role) -> Result<Client> {
        let mut conn = platform::connect(path).map_err(|source| Error::Connect { path: path.to_path_buf(), source })?;
        let hello = ClientHello::Hello(Hello { protocol: Range::ours(), client: client.into(), role });
        frame::write_json(&mut conn, &hello)?;
        let welcome = match read_json::<ServerHello>(&mut conn)? {
            ServerHello::Welcome(w) => w,
            ServerHello::Reject(r) => return Err(Error::Rejected(r)),
        };
        Ok(Client { conn, next_id: 1, welcome, events: Vec::new() })
    }

    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// The server has feature `cap` (`welcome.caps`).
    pub fn has(&self, cap: &str) -> bool {
        self.welcome.caps.iter().any(|c| c == cap)
    }

    /// Events that arrived while waiting for answers (and forget them).
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Call `method` and wait for its answer (which, for a request that
    /// needs the user's approval, comes once they decided).
    pub fn call(&mut self, method: &str, params: impl Serialize) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let params = serde_json::to_value(params).map_err(|e| Error::Protocol(e.to_string()))?;
        frame::write_json(&mut self.conn, &Request { id, method: method.into(), params })?;
        loop {
            match read_json::<ServerMsg>(&mut self.conn)? {
                ServerMsg::Response(r) if r.id == id => return r.into_result().map_err(Error::Server),
                ServerMsg::Response(r) => return Err(Error::Protocol(format!("answer to request {} while waiting for {id}", r.id))),
                ServerMsg::Event(e) => self.events.push(e),
            }
        }
    }

    /// [`call`](Self::call) with a typed result.
    pub fn call_as<R: DeserializeOwned>(&mut self, method: &str, params: impl Serialize) -> Result<R> {
        let v = self.call(method, params)?;
        serde_json::from_value(v).map_err(|e| Error::Protocol(format!("{method}: {e}")))
    }

    pub fn agents(&mut self) -> Result<Vec<AgentView>> {
        self.call_as(method::AGENT_LIST, Value::Null)
    }

    pub fn create_agent(&mut self, req: &AgentCreate) -> Result<AgentView> {
        self.call_as(method::AGENT_CREATE, req)
    }

    pub fn machines(&mut self) -> Result<Vec<ProviderMachines>> {
        self.call_as(method::MACHINE_LIST, Value::Null)
    }

    /// How new agents are made on one machine (its form and choices).
    pub fn create_form(&mut self, req: &FormRequest) -> Result<CreateForm> {
        self.call_as(method::MACHINE_FORM, req)
    }

    pub fn sessions(&mut self, filter: &SessionFilter) -> Result<Vec<ScannedPlace>> {
        self.call_as(method::SESSION_LIST, filter)
    }

    pub fn add_session(&mut self, req: &SessionAdd) -> Result<SessionAdded> {
        self.call_as(method::SESSION_ADD, req)
    }

    /// Pending approvals (verified UI clients only).
    pub fn approvals(&mut self) -> Result<Vec<ApprovalView>> {
        self.call_as(method::APPROVAL_LIST, Value::Null)
    }

    /// Answer an approval (verified UI clients only).
    pub fn answer_approval(&mut self, answer: &ApprovalAnswer) -> Result<()> {
        self.call(method::APPROVAL_ANSWER, answer).map(|_| ())
    }
}

/// The next JSON message (terminal and unknown frames are skipped).
fn read_json<T: DeserializeOwned>(r: &mut impl std::io::Read) -> Result<T> {
    loop {
        match frame::read(r)? {
            Some(Frame::Json(body)) => return serde_json::from_slice(&body).map_err(|e| Error::Protocol(e.to_string())),
            Some(_) => continue,
            None => return Err(Error::Protocol("Pitwall closed the connection".into())),
        }
    }
}
