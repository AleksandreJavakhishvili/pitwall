//! The per-share egress proxy (docs/spec/collaboration/sessions.md §6.3,
//! "Network"): a small HTTP proxy on `127.0.0.1` that a share's sessions
//! reach through `HTTPS_PROXY`/`HTTP_PROXY`, the only network their sandbox
//! allows. It forwards to the allowed hosts only (the kind's model hosts
//! from kind data plus hosts the owner added) and refuses everything else
//! with `403`, recording each refusal.
//!
//! - `CONNECT host:port` (HTTPS and anything tunnelled) and plain HTTP
//!   requests in absolute form (`GET http://host/…`), one request per
//!   connection;
//! - the proxy resolves names itself, so sessions need no DNS;
//! - a name that resolves to this machine (loopback, unspecified) is
//!   refused unless the rule names that address itself, so an allowed name
//!   can't be pointed at Pitwall's own local ports.
//!
//! Blocking, plain threads (one accepting, two per tunnel), like the rest
//! of the core.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use crate::error::{PwError, Result};

/// Longest request head accepted (request line and headers).
const MAX_HEAD: usize = 16 * 1024;
/// How long to wait for a client's request head, or to connect upstream.
const TIMEOUT: Duration = Duration::from_secs(15);
/// Refusals kept for [`EgressProxy::refusals`].
const KEEP_REFUSALS: usize = 256;

/// One allowed destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRule {
    /// Lower case. `*.example.com` matches subdomains of `example.com`
    /// (not `example.com` itself).
    host: String,
    /// `None`: 443 and 80.
    port: Option<u16>,
}

impl HostRule {
    /// `host`, `host:port`, `*.domain`, `[v6]:port` or an IP address.
    pub fn parse(rule: &str) -> Option<HostRule> {
        let rule = rule.trim().trim_end_matches('.').to_ascii_lowercase();
        if rule.is_empty() || rule.contains('/') || rule.contains(char::is_whitespace) {
            return None;
        }
        if rule.parse::<IpAddr>().is_ok() {
            return Some(HostRule { host: rule, port: None });
        }
        let (host, port) = split_host_port(&rule)?;
        if host.is_empty() || host == "*" || host == "*." {
            return None;
        }
        Some(HostRule { host, port })
    }

    fn matches(&self, host: &str, port: u16) -> bool {
        let port_ok = match self.port {
            Some(p) => p == port,
            None => port == 443 || port == 80,
        };
        let host_ok = match self.host.strip_prefix("*.") {
            Some(domain) => host.len() > domain.len() + 1 && host.ends_with(domain) && host[..host.len() - domain.len()].ends_with('.'),
            None => self.host == host,
        };
        port_ok && host_ok
    }

    /// The rule names an address of this machine itself (`localhost`,
    /// `127.0.0.1`), so connecting there is what the owner asked for.
    fn names_this_machine(&self) -> bool {
        self.host == "localhost" || self.host.parse::<IpAddr>().is_ok_and(|ip| local_ip(&ip))
    }
}

/// `host[:port]` or `[v6][:port]`; `None` when the port isn't a number.
fn split_host_port(s: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p.parse().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        return Some((host.to_string(), port));
    }
    match s.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some((host.to_string(), Some(port.parse().ok()?))),
        Some(_) => Some((s.to_string(), None)), // a bare IPv6 address
        None => Some((s.to_string(), None)),
    }
}

fn local_ip(ip: &IpAddr) -> bool {
    ip.is_loopback() || ip.is_unspecified()
}

/// A request the proxy turned down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub at: SystemTime,
    /// `host:port` as asked (or the raw target when it didn't parse).
    pub target: String,
    pub why: String,
}

type OnRefusal = Arc<dyn Fn(&Refusal) + Send + Sync>;

struct Shared {
    rules: RwLock<Vec<HostRule>>,
    refusals: Mutex<Vec<Refusal>>,
    on_refusal: Option<OnRefusal>,
    stop: AtomicBool,
}

impl Shared {
    fn refuse(&self, target: &str, why: &str) {
        let r = Refusal { at: SystemTime::now(), target: target.to_string(), why: why.to_string() };
        if let Some(f) = &self.on_refusal {
            f(&r);
        }
        let mut list = self.refusals.lock().unwrap_or_else(|e| e.into_inner());
        if list.len() >= KEEP_REFUSALS {
            list.remove(0);
        }
        list.push(r);
    }

    /// Where to connect for `host:port`, or why not.
    fn allow(&self, host: &str, port: u16) -> std::result::Result<Vec<SocketAddr>, String> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let rules = self.rules.read().unwrap_or_else(|e| e.into_inner());
        let Some(rule) = rules.iter().find(|r| r.matches(&host, port)) else {
            return Err("host not allowed".into());
        };
        let addrs: Vec<SocketAddr> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| format!("can't resolve: {e}"))?
            .filter(|a| rule.names_this_machine() || !local_ip(&a.ip()))
            .collect();
        if addrs.is_empty() {
            return Err("resolves to this machine".into());
        }
        Ok(addrs)
    }
}

/// A running proxy. Dropping it stops accepting; open tunnels end when
/// either side closes.
pub struct EgressProxy {
    port: u16,
    shared: Arc<Shared>,
}

impl EgressProxy {
    /// Listen on `127.0.0.1` (any free port) and allow `hosts` (see
    /// [`HostRule::parse`]; unparsable entries are an error).
    /// `on_refusal` is called for every refused request (logging).
    pub fn start(hosts: &[String], on_refusal: Option<OnRefusal>) -> Result<EgressProxy> {
        let rules = parse_rules(hosts)?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| PwError::other(format!("egress proxy: {e}")))?;
        let port = listener.local_addr().map_err(|e| PwError::other(e.to_string()))?.port();
        let shared = Arc::new(Shared {
            rules: RwLock::new(rules),
            refusals: Mutex::new(Vec::new()),
            on_refusal,
            stop: AtomicBool::new(false),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("pitwall-egress".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if s.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    let s = s.clone();
                    let _ = std::thread::Builder::new().name("pitwall-egress-conn".into()).spawn(move || {
                        let _ = serve(&s, conn);
                    });
                }
            })
            .map_err(|e| PwError::other(format!("egress proxy: {e}")))?;
        Ok(EgressProxy { port, shared })
    }

    /// The port on `127.0.0.1` (`sandbox::Egress::proxy_port`).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>`, for `HTTPS_PROXY`/`HTTP_PROXY`.
    pub fn url(&self) -> String {
        proxy_url(self.port)
    }

    /// Replace the allowed hosts (the owner edited the share); applies to
    /// new requests.
    pub fn set_hosts(&self, hosts: &[String]) -> Result<()> {
        let rules = parse_rules(hosts)?;
        *self.shared.rules.write().unwrap_or_else(|e| e.into_inner()) = rules;
        Ok(())
    }

    /// The latest refusals, oldest first.
    pub fn refusals(&self) -> Vec<Refusal> {
        self.shared.refusals.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for EgressProxy {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // Wake the accepting thread so it sees `stop`.
        let _ = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), Duration::from_secs(1));
    }
}

/// The proxy URL for a port.
pub fn proxy_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn parse_rules(hosts: &[String]) -> Result<Vec<HostRule>> {
    hosts
        .iter()
        .map(|h| HostRule::parse(h).ok_or_else(|| PwError::other(format!("not a host: {h:?}"))))
        .collect()
}

/// One client connection: read the request head, check, tunnel or forward.
fn serve(shared: &Shared, client: TcpStream) -> io::Result<()> {
    client.set_read_timeout(Some(TIMEOUT))?;
    let mut reader = BufReader::new(client.try_clone()?);
    let mut head = Vec::new();
    loop {
        let n = reader.by_ref().take((MAX_HEAD - head.len()) as u64).read_until(b'\n', &mut head)?;
        if n == 0 || head.len() >= MAX_HEAD {
            return reply(&client, "400 Bad Request", "incomplete request");
        }
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let (method, target, version) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or("HTTP/1.1"));

    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, Some(port))) = split_host_port(target) else {
            shared.refuse(target, "bad CONNECT target");
            return reply(&client, "400 Bad Request", "bad CONNECT target");
        };
        let upstream = match open(shared, &host, port, target) {
            Ok(u) => u,
            Err(status) => return reply(&client, status.0, &status.1),
        };
        (&client).write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?;
        // Bytes the client sent after the head (rare before the reply).
        let early = reader.buffer().to_vec();
        if !early.is_empty() {
            (&upstream).write_all(&early)?;
        }
        return splice(client, upstream);
    }

    // Plain HTTP in absolute form: http://host[:port]/path
    let Some(rest) = target.strip_prefix("http://").or_else(|| target.strip_prefix("HTTP://")) else {
        shared.refuse(target, "not a proxy request");
        return reply(&client, "400 Bad Request", "only CONNECT and absolute http:// requests");
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let Some((host, port)) = split_host_port(authority) else {
        shared.refuse(target, "bad URL");
        return reply(&client, "400 Bad Request", "bad URL");
    };
    let port = port.unwrap_or(80);
    let upstream = match open(shared, &host, port, &format!("{host}:{port}")) {
        Ok(u) => u,
        Err(status) => return reply(&client, status.0, &status.1),
    };
    let mut out = format!("{method} {path} {version}\r\n");
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if name.starts_with("proxy-") || name == "connection" || name == "keep-alive" {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    (&upstream).write_all(out.as_bytes())?;
    let early = reader.buffer().to_vec();
    if !early.is_empty() {
        (&upstream).write_all(&early)?;
    }
    splice(client, upstream)
}

/// Connect upstream if allowed; else record the refusal and say what to
/// answer.
fn open(shared: &Shared, host: &str, port: u16, target: &str) -> std::result::Result<TcpStream, (&'static str, String)> {
    let addrs = match shared.allow(host, port) {
        Ok(a) => a,
        Err(why) => {
            shared.refuse(target, &why);
            return Err(("403 Forbidden", format!("Pitwall's egress proxy refused {target}: {why}")));
        }
    };
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, TIMEOUT) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    let why = last.map(|e| e.to_string()).unwrap_or_default();
    Err(("502 Bad Gateway", format!("can't reach {target}: {why}")))
}

fn reply(mut client: &TcpStream, status: &str, body: &str) -> io::Result<()> {
    let msg = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(msg.as_bytes())?;
    let _ = client.shutdown(Shutdown::Both);
    Ok(())
}

/// Copy both ways until either side closes.
fn splice(client: TcpStream, upstream: TcpStream) -> io::Result<()> {
    client.set_read_timeout(None)?;
    let (c2, u2) = (client.try_clone()?, upstream.try_clone()?);
    let up = std::thread::Builder::new().name("pitwall-egress-up".into()).spawn(move || {
        let _ = io::copy(&mut &c2, &mut &u2);
        let _ = u2.shutdown(Shutdown::Write);
    })?;
    let _ = io::copy(&mut &upstream, &mut &client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = up.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-shot HTTP server on localhost answering `body`.
    pub(crate) fn http_server(body: &'static str) -> u16 {
        let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let _ = c.read(&mut buf);
                let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        port
    }

    fn ask(proxy: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", proxy)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn rules_parse_and_match() {
        let r = HostRule::parse("API.Anthropic.com").unwrap();
        assert!(r.matches("api.anthropic.com", 443));
        assert!(r.matches("api.anthropic.com", 80));
        assert!(!r.matches("api.anthropic.com", 22));
        assert!(!r.matches("evil-api.anthropic.com", 443));
        let w = HostRule::parse("*.googleapis.com").unwrap();
        assert!(w.matches("oauth2.googleapis.com", 443));
        assert!(!w.matches("googleapis.com", 443));
        assert!(!w.matches("xgoogleapis.com", 443));
        let p = HostRule::parse("registry.npmjs.org:8443").unwrap();
        assert!(p.matches("registry.npmjs.org", 8443) && !p.matches("registry.npmjs.org", 443));
        assert!(HostRule::parse("127.0.0.1:9").unwrap().names_this_machine());
        assert!(HostRule::parse("[::1]:9").unwrap().matches("::1", 9));
        for bad in ["", "*", "a b", "http://x", "x:notaport"] {
            assert!(HostRule::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn tunnels_to_allowed_hosts_and_refuses_the_rest() {
        let target = http_server("hello");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let proxy = EgressProxy::start(
            &[format!("127.0.0.1:{target}")],
            Some(Arc::new(move |r: &Refusal| log.lock().unwrap().push(r.target.clone()))),
        )
        .unwrap();

        let ok = ask(proxy.port(), &format!("CONNECT 127.0.0.1:{target} HTTP/1.1\r\nHost: x\r\n\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert!(ok.starts_with("HTTP/1.1 200 Connection established"), "{ok}");
        assert!(ok.ends_with("hello"), "{ok}");

        let plain = ask(proxy.port(), &format!("GET http://127.0.0.1:{target}/x HTTP/1.1\r\nHost: x\r\nProxy-Authorization: y\r\n\r\n"));
        assert!(plain.starts_with("HTTP/1.1 200 OK") && plain.ends_with("hello"), "{plain}");

        let no = ask(proxy.port(), "CONNECT example.com:443 HTTP/1.1\r\n\r\n");
        assert!(no.starts_with("HTTP/1.1 403"), "{no}");
        let other_port = ask(proxy.port(), &format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", target.wrapping_add(1)));
        assert!(other_port.starts_with("HTTP/1.1 403"), "{other_port}");
        let not_proxy = ask(proxy.port(), "GET / HTTP/1.1\r\n\r\n");
        assert!(not_proxy.starts_with("HTTP/1.1 400"), "{not_proxy}");

        let refused: Vec<String> = proxy.refusals().into_iter().map(|r| r.target).collect();
        assert_eq!(refused[..2], ["example.com:443".to_string(), format!("127.0.0.1:{}", target.wrapping_add(1))]);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn names_resolving_to_this_machine_are_refused() {
        let target = http_server("local");
        let proxy = EgressProxy::start(&[format!("localhost:{target}")], None).unwrap();
        // "localhost" names this machine on purpose: allowed.
        let ok = ask(proxy.port(), &format!("CONNECT localhost:{target} HTTP/1.1\r\n\r\nGET / HTTP/1.1\r\n\r\n"));
        assert!(ok.starts_with("HTTP/1.1 200") && ok.ends_with("local"), "{ok}");
        proxy.set_hosts(&[format!("*.localhost:{target}")]).unwrap();
        // A name that merely resolves here (wildcard rule) is not.
        let no = ask(proxy.port(), &format!("CONNECT a.localhost:{target} HTTP/1.1\r\n\r\n"));
        assert!(no.starts_with("HTTP/1.1 403"), "{no}");
        assert!(proxy.set_hosts(&["not a host".into()]).is_err());
    }
}
