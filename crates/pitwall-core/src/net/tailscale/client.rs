//! Outbound HTTP to a tailnet device (network.md §15.5 "client"): `ureq`,
//! blocking, plain HTTP inside WireGuard, straight to the device's
//! Tailscale IP as `status` lists it for its `StableID` (no DNS, so a
//! name can't be pointed elsewhere, and MagicDNS may be off).
//!
//! [`Timeouts`] are honoured per request: `connect` bounds reaching the
//! device, `idle` bounds each wait for bytes (headers or body), so an SSE
//! stream with `idle: None` waits as long as it stays open. ureq's own
//! timeouts are totals, so a small connector of ours sets the socket's
//! read timeout instead (ureq's `unversioned` transport API, hence the
//! exact version pin).

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::time::Duration as UDuration;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport,
};

use super::{ip_host, lock, Inner};
use crate::error::{PwError, Result};
use crate::net::{http, Body, DeviceAddr, HttpClient, Request, Response, Timeouts};

/// Responses with a length up to this are read whole ([`Body::Bytes`]);
/// longer or unlength'd ones stream.
const READ_WHOLE: u64 = 4 * 1024 * 1024;

/// One ureq agent (and its connection pool) per distinct [`Timeouts`].
#[derive(Default)]
pub(crate) struct Agents(Mutex<Vec<(Timeouts, ureq::Agent)>>);

impl Agents {
    fn get(&self, inner: &Inner, timeouts: Timeouts) -> ureq::Agent {
        let mut agents = lock(&self.0);
        if let Some((_, agent)) = agents.iter().find(|(t, _)| *t == timeouts) {
            return agent.clone();
        }
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .timeout_connect(Some(timeouts.connect))
            .user_agent(format!("pitwall/{}", env!("CARGO_PKG_VERSION")))
            .build();
        let connector = TailnetConnector { idle: timeouts.idle, inner: inner.me.clone() };
        let agent = ureq::Agent::with_parts(config, connector, DefaultResolver::default());
        agents.push((timeouts, agent.clone()));
        agent
    }
}

/// [`Network::client`](crate::net::Network::client).
pub(crate) fn to(inner: &Arc<Inner>, to: &DeviceAddr) -> Result<Box<dyn HttpClient>> {
    if to.network != inner.id() {
        return Err(PwError::unreachable(format!("{} is not on {}", to.node, inner.id())));
    }
    Ok(Box::new(TailnetClient { inner: inner.clone(), node: to.node.clone() }))
}

struct TailnetClient {
    inner: Arc<Inner>,
    node: String,
}

impl TailnetClient {
    /// Where the device is right now.
    fn target(&self) -> Result<(SocketAddr, String)> {
        let status = self.inner.ready()?;
        let peer = status
            .peer(&self.node)
            .ok_or_else(|| PwError::unreachable(format!("{} isn't on the tailnet", self.node)))?;
        let label = peer.label();
        if !peer.online {
            return Err(PwError::unreachable(format!("{label} is offline")));
        }
        let ip = pick_ip(&peer.tailscale_ips, |ip| self.inner.address_ok(ip))
            .ok_or_else(|| PwError::unreachable(format!("{label} has no tailnet address")))?;
        let addr = self.route(ip).ok_or_else(|| PwError::unreachable(format!("{label} refused the connection")))?;
        Ok((addr, label))
    }

    /// Tests: every device listens on 127.0.0.1, on a port of its own.
    #[cfg(any(test, feature = "testing"))]
    fn route(&self, ip: IpAddr) -> Option<SocketAddr> {
        match &self.inner.config.test {
            Some(hooks) => hooks.port_of(&self.node).map(|port| SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port)),
            None => Some(SocketAddr::new(ip, self.inner.config.port)),
        }
    }

    #[cfg(not(any(test, feature = "testing")))]
    fn route(&self, ip: IpAddr) -> Option<SocketAddr> {
        Some(SocketAddr::new(ip, self.inner.config.port))
    }
}

/// IPv4 first (every tailnet node has one), then IPv6.
fn pick_ip(ips: &[IpAddr], ok: impl Fn(IpAddr) -> bool) -> Option<IpAddr> {
    ips.iter().copied().filter(|ip| ok(*ip)).min_by_key(|ip| ip.is_ipv6())
}

impl HttpClient for TailnetClient {
    fn send(&self, req: Request) -> Result<Response> {
        let (addr, label) = self.target()?;
        let timeouts = req.extensions().get::<Timeouts>().copied().unwrap_or_default();
        let (mut parts, body) = req.into_parts();
        let path = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_string();
        let uri: http::Uri = format!("http://{}:{}{path}", ip_host(&addr.ip()), addr.port())
            .parse()
            .map_err(|e| PwError::other(format!("bad request path {path}: {e}")))?;
        parts.uri = uri;
        // The connection's business, and Host is ureq's.
        for name in ["host", "connection", "content-length", "transfer-encoding"] {
            parts.headers.remove(name);
        }
        parts.extensions = http::Extensions::new();
        let agent = self.inner.clients.get(&self.inner, timeouts);
        let fail = |e: ureq::Error| PwError::unreachable(format!("{label}: {e}"));
        let res = match body {
            Body::Empty => agent.run(http::Request::from_parts(parts, ())).map_err(fail)?,
            Body::Bytes(bytes) => agent.run(http::Request::from_parts(parts, bytes)).map_err(fail)?,
            Body::Stream(reader) => {
                let body = ureq::SendBody::from_owned_reader(SendReader(reader));
                agent.run(http::Request::from_parts(parts, body)).map_err(fail)?
            }
        };
        let (parts, body) = res.into_parts();
        let length = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let body = match length {
            Some(0) => Body::Empty,
            Some(n) if n <= READ_WHOLE => {
                let mut bytes = Vec::with_capacity(n as usize);
                body.into_reader().read_to_end(&mut bytes).map_err(|e| PwError::unreachable(format!("{label}: {e}")))?;
                Body::Bytes(bytes)
            }
            _ => Body::Stream(Box::new(body.into_reader())),
        };
        Ok(http::Response::from_parts(parts, body))
    }
}

/// ureq wants `Read + 'static`; ours is also `Send`.
struct SendReader(Box<dyn Read + Send>);

impl Read for SendReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

// --------------------------------------------------------------- transport

/// Connects to the address ureq resolved from the URL (always an IP
/// literal here) and hands ureq a transport whose reads time out after
/// `idle` of silence.
struct TailnetConnector {
    idle: Option<Duration>,
    inner: Weak<Inner>,
}

impl fmt::Debug for TailnetConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TailnetConnector").field("idle", &self.idle).finish()
    }
}

impl Connector<()> for TailnetConnector {
    type Out = IdleTransport;

    fn connect(&self, details: &ConnectionDetails, _chained: Option<()>) -> std::result::Result<Option<IdleTransport>, ureq::Error> {
        let addr = *details.addrs.first().ok_or(ureq::Error::HostNotFound)?;
        let stream = match details.timeout.not_zero() {
            Some(t) => TcpStream::connect_timeout(&addr, *t),
            None => TcpStream::connect(addr),
        }
        .map_err(|e| match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ureq::Error::Timeout(ureq::Timeout::Connect),
            _ => ureq::Error::Io(e),
        })?;
        stream.set_nodelay(true)?;
        if let Some(inner) = self.inner.upgrade() {
            connected(&inner, &stream);
        }
        let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
        Ok(Some(IdleTransport { stream, buffers, idle: self.idle }))
    }
}

/// Tests: tell the scripted tailnet who this connection is from, before a
/// byte is sent (the kernel and Tailscale know it in production).
#[cfg(any(test, feature = "testing"))]
fn connected(inner: &Inner, stream: &TcpStream) {
    let Some(hooks) = &inner.config.test else { return };
    let me = inner.status().ok().and_then(|s| s.self_).map(|s| s.id);
    if let (Some(me), Ok(local)) = (me, stream.local_addr()) {
        hooks.connected(&me, local);
    }
}

#[cfg(not(any(test, feature = "testing")))]
fn connected(_inner: &Inner, _stream: &TcpStream) {}

/// ureq's TCP transport, plus the idle bound on reads.
#[derive(Debug)]
struct IdleTransport {
    stream: TcpStream,
    buffers: LazyBuffers,
    idle: Option<Duration>,
}

/// A read or write that timed out (`WouldBlock` on Unix, `TimedOut` on
/// Windows) is ureq's timeout.
fn io_error(e: io::Error, reason: ureq::Timeout) -> ureq::Error {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ureq::Error::Timeout(reason),
        _ => ureq::Error::Io(e),
    }
}

impl Transport for IdleTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> std::result::Result<(), ureq::Error> {
        self.stream.set_write_timeout(timeout.not_zero().map(|t| *t))?;
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output).map_err(|e| io_error(e, timeout.reason))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> std::result::Result<bool, ureq::Error> {
        let timeout = match self.idle {
            Some(idle) if timeout.after > UDuration::from(idle) => {
                NextTimeout { after: UDuration::from(idle), reason: ureq::Timeout::RecvBody }
            }
            _ => timeout,
        };
        self.stream.set_read_timeout(timeout.not_zero().map(|t| *t))?;
        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input).map_err(|e| io_error(e, timeout.reason))?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        // Open when a read would block: nothing unasked-for waiting, no EOF.
        if self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let mut buf = [0u8; 1];
        let open = matches!(self.stream.read(&mut buf), Err(e) if e.kind() == io::ErrorKind::WouldBlock);
        self.stream.set_nonblocking(false).is_ok() && open
    }
}
