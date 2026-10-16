//! The listener (network.md §15.5, tailnet-peers.md §4 "Rules for the
//! listener"):
//!
//! - bound only to this node's Tailscale IPs, one socket per address
//!   (never `0.0.0.0`, `::` or a LAN address), rebound when they change;
//! - on accept, the source must be in the tailnet ranges and `whois` must
//!   name a node, before a byte is read; otherwise the connection is
//!   closed without a byte;
//! - every request is proven again (cached `whois`), so a node removed
//!   while connected is refused on its next request, and its connections
//!   are closed when the poller notices;
//! - per-caller limits (protocol.md §8.1): connections, requests per second,
//!   body size, open streams.
//!
//! One thread accepts per address; each connection has a thread of its own
//! (blocking, no async runtime).

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::http1::{self, ReadError};
use super::{lock, Inner, Limits};
use crate::error::{ErrorCode, PwError, Result};
use crate::net::{Caller, Handler, PersonId, Revocation, ServeHandle};

/// A handler being served: its listeners and open connections.
pub(crate) struct Serving {
    token: u64,
    handler: Arc<dyn Handler>,
    port: u16,
    listeners: Mutex<Vec<Listener>>,
    conns: Arc<Conns>,
}

struct Listener {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// How often a listener looks for a stop between connections. (Waking a
/// blocking accept by connecting to it doesn't work everywhere: macOS
/// doesn't route a node's own Tailscale IPv6 address back to it.)
const ACCEPT_POLL: Duration = Duration::from_millis(50);

impl Listener {
    /// Close the socket; returns once nothing listens on it any more.
    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start serving `handler` (Network::serve).
pub(crate) fn start(inner: &Arc<Inner>, handler: Arc<dyn Handler>) -> Result<ServeHandle> {
    let status = inner.ready()?;
    let me = status.self_.clone().ok_or_else(|| PwError::unreachable("Tailscale has no address for this device"))?;
    let token = {
        let mut next = lock(&inner.next_serve);
        *next += 1;
        *next
    };
    {
        let mut slot = lock(&inner.serving);
        if slot.is_some() {
            return Err(PwError::new(ErrorCode::Conflict, "already serving on Tailscale"));
        }
        let ips = bind_ips(inner, &me.tailscale_ips);
        let port = if inner.testing() { 0 } else { inner.config.port };
        let serving = Serving {
            token,
            handler,
            port,
            listeners: Mutex::new(Vec::new()),
            conns: Arc::new(Conns::new(inner.config.limits)),
        };
        let bound = serving.bind(inner, &ips)?;
        test_bound(inner, &me.id, &bound);
        *slot = Some(serving);
    }
    super::watch::ensure(inner);
    let weak: Weak<Inner> = Arc::downgrade(inner);
    Ok(ServeHandle::new(move || {
        let Some(inner) = weak.upgrade() else { return };
        let serving = {
            let mut slot = lock(&inner.serving);
            if slot.as_ref().is_some_and(|s| s.token == token) { slot.take() } else { None }
        };
        if let Some(serving) = serving {
            serving.stop();
        }
    }))
}

/// The addresses to listen on: this node's tailnet IPs (127.0.0.1 in tests).
fn bind_ips(inner: &Inner, mine: &[IpAddr]) -> Vec<IpAddr> {
    if inner.testing() {
        return vec![IpAddr::from([127, 0, 0, 1])];
    }
    mine.iter().copied().filter(|ip| super::is_tailnet_ip(*ip)).collect()
}

#[cfg(any(test, feature = "testing"))]
fn test_bound(inner: &Inner, node: &str, addrs: &[SocketAddr]) {
    if let (Some(hooks), Some(addr)) = (&inner.config.test, addrs.first()) {
        hooks.bound(node, *addr);
    }
}

#[cfg(not(any(test, feature = "testing")))]
fn test_bound(_inner: &Inner, _node: &str, _addrs: &[SocketAddr]) {}

impl Serving {
    pub(crate) fn addrs(&self) -> Vec<SocketAddr> {
        lock(&self.listeners).iter().map(|l| l.addr).collect()
    }

    pub(crate) fn port(&self) -> u16 {
        self.addrs().first().map_or(self.port, |a| a.port())
    }

    /// Listen on `ips` (one socket each). Fails only when none binds.
    fn bind(&self, inner: &Inner, ips: &[IpAddr]) -> Result<Vec<SocketAddr>> {
        if ips.is_empty() {
            return Err(PwError::unreachable("this device has no Tailscale address to listen on"));
        }
        let mut errors = Vec::new();
        let mut bound = Vec::new();
        let mut port = self.port();
        for ip in ips {
            match TcpListener::bind(SocketAddr::new(*ip, port)) {
                Ok(listener) => {
                    let Ok(addr) = listener.local_addr() else { continue };
                    // One port on every address, even when the OS picked it.
                    port = addr.port();
                    let stop = Arc::new(AtomicBool::new(false));
                    let thread = self.accept(inner, listener, stop.clone());
                    lock(&self.listeners).push(Listener { addr, stop, thread });
                    bound.push(addr);
                }
                Err(e) => errors.push(format!("{ip}: {e}")),
            }
        }
        if bound.is_empty() {
            return Err(PwError::unreachable(format!("can't listen on port {port}: {}", errors.join("; "))));
        }
        Ok(bound)
    }

    /// Listen on the new addresses only.
    pub(crate) fn rebind(&self, inner: &Inner, mine: &[IpAddr]) {
        let want = bind_ips(inner, mine);
        let mut listeners = lock(&self.listeners);
        let (keep, gone): (Vec<Listener>, Vec<Listener>) =
            listeners.drain(..).partition(|l| want.contains(&l.addr.ip()));
        *listeners = keep;
        let have: Vec<IpAddr> = listeners.iter().map(|l| l.addr.ip()).collect();
        drop(listeners);
        for l in gone {
            l.stop();
        }
        let new: Vec<IpAddr> = want.into_iter().filter(|ip| !have.contains(ip)).collect();
        if !new.is_empty() {
            let _ = self.bind(inner, &new);
        }
    }

    fn accept(&self, inner: &Inner, listener: TcpListener, stop: Arc<AtomicBool>) -> Option<std::thread::JoinHandle<()>> {
        // Accept threads own the backend until serving stops.
        let inner = inner.me.upgrade()?;
        let handler = self.handler.clone();
        let conns = self.conns.clone();
        listener.set_nonblocking(true).ok()?;
        let spawned = std::thread::Builder::new().name("pw-collab-accept".into()).spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(_) => {
                        // Nothing waiting (or a transient error): look again soon.
                        std::thread::sleep(ACCEPT_POLL);
                        continue;
                    }
                };
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                // Closed without a byte: not from the tailnet, or too many.
                let Ok(peer) = stream.peer_addr() else { continue };
                if !inner.address_ok(peer.ip()) {
                    continue;
                }
                let Some(slot) = Conns::enter_total(&conns) else { continue };
                let (inner, handler, conns) = (inner.clone(), handler.clone(), conns.clone());
                let _ = std::thread::Builder::new()
                    .name("pw-collab-conn".into())
                    .spawn(move || connection(&inner, &*handler, &conns, stream, peer, slot));
            }
        });
        spawned.ok()
    }

    /// Close the connections of someone the provider removed.
    pub(crate) fn close(&self, revocation: &Revocation) {
        self.conns.close(|c| match revocation {
            Revocation::Device { device } => c.device == device.node,
            Revocation::Identity { identity } => c.identity.is(identity),
        });
    }

    fn stop(&self) {
        let listeners: Vec<Listener> = lock(&self.listeners).drain(..).collect();
        for l in listeners {
            l.stop();
        }
        self.conns.close(|_| true);
    }
}

/// One connection, from accept to close.
fn connection(inner: &Inner, handler: &dyn Handler, conns: &Conns, stream: TcpStream, peer: SocketAddr, _slot: TotalSlot) {
    let limits = conns.limits;
    let _ = stream.set_read_timeout(Some(limits.idle));
    let _ = stream.set_write_timeout(Some(limits.idle));
    let _ = stream.set_nodelay(true);
    // Proven before a byte is read; otherwise closed without one.
    let caller = match inner.caller(peer, handler) {
        Ok(Some(caller)) => caller,
        _ => {
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    };
    let person = caller.person().clone();
    let Some(_person_slot) = conns.enter_person(&person) else {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    };
    let (Ok(read_half), Ok(control)) = (stream.try_clone(), stream.try_clone()) else { return };
    let registered = conns.register(&caller, control);
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(stream);

    loop {
        let head = match http1::read_head(&mut reader, limits.max_head) {
            Ok(Some(head)) => head,
            Ok(None) | Err(ReadError::Closed) => break,
            Err(ReadError::TooLarge) => {
                let _ = http1::write_response(&mut writer, http1::status_only(431, "request head too large"), false, false);
                break;
            }
            Err(ReadError::Bad) => {
                let _ = http1::write_response(&mut writer, http1::status_only(400, "bad request"), false, false);
                break;
            }
        };
        // Still the same, still on the tailnet?
        let caller = match inner.caller(peer, handler) {
            Ok(Some(now)) if now.identity().is(caller.identity()) && now.device() == caller.device() => now,
            _ => break,
        };
        if conns.closed(registered) {
            break;
        }
        if !conns.take_request(&person) {
            let _ = http1::write_response(&mut writer, http1::status_only(429, "too many requests"), false, false);
            break;
        }
        let body = match http1::read_body(&head, &mut reader, &mut writer, limits.max_body) {
            Ok(body) => body,
            Err(ReadError::TooLarge) => {
                let _ = http1::write_response(&mut writer, http1::status_only(413, "request body too large"), false, false);
                // Let a client still sending see the answer, not a reset.
                linger(&mut reader, limits.max_body);
                break;
            }
            Err(ReadError::Bad) => {
                let _ = http1::write_response(&mut writer, http1::status_only(400, "bad request"), false, false);
                break;
            }
            Err(ReadError::Closed) => break,
        };
        let Ok(request) = http1::request(&head, body) else {
            let _ = http1::write_response(&mut writer, http1::status_only(400, "bad request"), false, false);
            break;
        };
        let keep_alive = head.keep_alive();
        let head_only = head.method == "HEAD";
        let mut response = handler.handle(request, caller);
        let stream_slot = if response.body().is_stream() {
            let slot = conns.enter_stream(&person);
            if slot.is_none() {
                // Dropping the body tells its writer the reader is gone.
                response = http1::status_only(429, "too many open streams");
            }
            slot
        } else {
            None
        };
        let written = http1::write_response(&mut writer, response, keep_alive, head_only);
        drop(stream_slot);
        if written.is_err() || !keep_alive || conns.closed(registered) {
            break;
        }
    }
    conns.unregister(registered);
    if let Ok(s) = writer.into_inner() {
        let _ = s.shutdown(Shutdown::Both);
    }
}

/// Read and drop what the peer still sends, up to `max` bytes or a second.
fn linger(reader: &mut BufReader<TcpStream>, max: usize) {
    let _ = reader.get_ref().set_read_timeout(Some(Duration::from_secs(1)));
    let _ = std::io::copy(&mut std::io::Read::take(reader, max as u64 * 2), &mut std::io::sink());
}

// ------------------------------------------------------------------ limits

/// Open connections and what each caller uses.
struct Conns {
    limits: Limits,
    total: AtomicUsize,
    next: AtomicU64,
    open: Mutex<HashMap<u64, Open>>,
    people: Mutex<HashMap<PersonId, Usage>>,
}

struct Open {
    identity: crate::net::Identity,
    device: String,
    stream: TcpStream,
    closed: bool,
}

struct Usage {
    conns: usize,
    streams: usize,
    tokens: f64,
    at: Instant,
}

/// A connection counted against `total_connections`, given back on drop.
struct TotalSlot(Arc<Conns>);

impl Drop for TotalSlot {
    fn drop(&mut self) {
        self.0.total.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A caller's connection or stream, given back on drop.
struct PersonSlot<'a> {
    conns: &'a Conns,
    person: PersonId,
    stream: bool,
}

impl Drop for PersonSlot<'_> {
    fn drop(&mut self) {
        let mut people = lock(&self.conns.people);
        if let Some(u) = people.get_mut(&self.person) {
            if self.stream {
                u.streams = u.streams.saturating_sub(1);
            } else {
                u.conns = u.conns.saturating_sub(1);
            }
        }
    }
}

impl Conns {
    fn new(limits: Limits) -> Conns {
        Conns {
            limits,
            total: AtomicUsize::new(0),
            next: AtomicU64::new(1),
            open: Mutex::new(HashMap::new()),
            people: Mutex::new(HashMap::new()),
        }
    }

    fn enter_total(conns: &Arc<Conns>) -> Option<TotalSlot> {
        let before = conns.total.fetch_add(1, Ordering::SeqCst);
        let slot = TotalSlot(conns.clone());
        (before < conns.limits.total_connections).then_some(slot)
    }

    fn usage<'a>(&self, people: &'a mut HashMap<PersonId, Usage>, person: &PersonId) -> &'a mut Usage {
        if people.len() > 4096 {
            let full = self.limits.burst as f64;
            people.retain(|_, u| u.conns > 0 || u.streams > 0 || u.tokens < full);
        }
        people.entry(person.clone()).or_insert_with(|| Usage {
            conns: 0,
            streams: 0,
            tokens: self.limits.burst as f64,
            at: Instant::now(),
        })
    }

    fn enter_person(&self, person: &PersonId) -> Option<PersonSlot<'_>> {
        let mut people = lock(&self.people);
        let usage = self.usage(&mut people, person);
        if usage.conns >= self.limits.connections {
            return None;
        }
        usage.conns += 1;
        Some(PersonSlot { conns: self, person: person.clone(), stream: false })
    }

    fn enter_stream(&self, person: &PersonId) -> Option<PersonSlot<'_>> {
        let mut people = lock(&self.people);
        let usage = self.usage(&mut people, person);
        if usage.streams >= self.limits.streams {
            return None;
        }
        usage.streams += 1;
        Some(PersonSlot { conns: self, person: person.clone(), stream: true })
    }

    /// One request's token from the caller's bucket.
    fn take_request(&self, person: &PersonId) -> bool {
        let mut people = lock(&self.people);
        let (rate, burst) = (self.limits.requests_per_sec as f64, self.limits.burst as f64);
        let usage = self.usage(&mut people, person);
        let now = Instant::now();
        usage.tokens = (usage.tokens + now.duration_since(usage.at).as_secs_f64() * rate).min(burst);
        usage.at = now;
        if usage.tokens < 1.0 {
            return false;
        }
        usage.tokens -= 1.0;
        true
    }

    fn register(&self, caller: &Caller, stream: TcpStream) -> u64 {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let device = caller.device().map(|d| d.node.clone()).unwrap_or_default();
        lock(&self.open).insert(id, Open { identity: caller.identity().clone(), device, stream, closed: false });
        id
    }

    fn unregister(&self, id: u64) {
        lock(&self.open).remove(&id);
    }

    fn closed(&self, id: u64) -> bool {
        lock(&self.open).get(&id).is_none_or(|o| o.closed)
    }

    /// Close every open connection `which` picks: its reads and writes fail
    /// at once, streams included.
    fn close(&self, which: impl Fn(&Open) -> bool) {
        for open in lock(&self.open).values_mut().filter(|o| which(o)) {
            open.closed = true;
            let _ = open.stream.shutdown(Shutdown::Both);
        }
    }
}
