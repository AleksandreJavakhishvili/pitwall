//! `net` (wit/net.wit, permissions.md §5.1–5.2): requests a tile asks
//! Pitwall to make. The host checks each one at the boundary (the URL
//! parsed here, IDNA applied by `url`, the host compared with the grant
//! before any name lookup) and hands it to a [`Fetcher`] on a thread of
//! its own; the answer comes back to the tile's worker as a `fetched`
//! event. [`HttpFetcher`] is Pitwall's (ureq with rustls, no cookies,
//! proxy or credentials, redirects followed here and checked again); the
//! test harness puts in its own ([`TileHost::set_fetcher`]).
//!
//! [`TileHost::set_fetcher`]: crate::TileHost::set_fetcher

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use pitwall_core::extensions::net::{allows, HostPattern};

pub use crate::bindings::pitwall::tile::net::{FetchError, Fetched, Header, Request, Response};
use crate::limits::{FETCH_TIMEOUT, MAX_FETCH_TIMEOUT, MAX_REDIRECTS, MAX_REQUEST_BODY, MAX_RESPONSE_BODY};

/// The methods a request may use.
pub const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// Headers Pitwall sets or never sends for a tile.
fn forbidden_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(n.as_str(), "host" | "content-length" | "connection" | "cookie" | "cookie2" | "transfer-encoding" | "te" | "upgrade" | "keep-alive" | "expect") || n.starts_with("proxy-")
}

/// Where a tile's requests may go: the hosts of its `net` grant, and its
/// own server's address (services.md §13.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reach {
    /// The `net` grant's hosts (empty without the grant).
    pub hosts: Vec<HostPattern>,
    /// The extension's own server: `127.0.0.1:8790`, over http.
    pub own_server: Option<std::net::SocketAddr>,
}

impl Reach {
    /// Whether `url` may be requested; else the host it names, for the
    /// refusal.
    pub fn check(&self, url: &url::Url) -> Result<(), String> {
        let host = match url.host() {
            Some(url::Host::Domain(d)) => d.to_string(),
            Some(url::Host::Ipv4(a)) => a.to_string(),
            Some(url::Host::Ipv6(a)) => a.to_string(),
            None => return Err(url.as_str().to_string()),
        };
        let port = url.port_or_known_default().unwrap_or(0);
        if allows(&self.hosts, url.scheme(), &host, port) {
            return Ok(());
        }
        if let Some(own) = self.own_server {
            // Its own address, or `localhost` for it; http only (a
            // service's `ready.listen` is on this computer only).
            let ip = own.ip();
            let same_host = ip.is_loopback() && (host == ip.to_string() || host == "localhost");
            if url.scheme() == "http" && same_host && port == own.port() {
                return Ok(());
            }
        }
        Err(match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host,
        })
    }
}

/// A request checked by the host, ready to send.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub method: String,
    pub url: url::Url,
    pub headers: Vec<Header>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

/// Check a tile's request against its reach and the limits.
pub(crate) fn check(req: Request, reach: &Reach) -> Result<Outgoing, FetchError> {
    let method = req.method.to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(FetchError::Invalid(format!("method “{}” isn't one of {}", crate::host::cut(&req.method, 20), METHODS.join(", "))));
    }
    let url = url::Url::parse(&req.url).map_err(|e| FetchError::Invalid(format!("“{}” isn't an address: {e}", crate::host::cut(&req.url, 200))))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(FetchError::Invalid(format!("“{}” must start with https:// (http:// only to this computer)", crate::host::cut(&req.url, 200))));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchError::Invalid("an address can't carry a user name or password: send them in a header".into()));
    }
    reach.check(&url).map_err(FetchError::Denied)?;
    for h in &req.headers {
        if forbidden_header(&h.name) {
            return Err(FetchError::Invalid(format!("header “{}” is set by Pitwall", crate::host::cut(&h.name, 60))));
        }
        let ok = !h.name.is_empty() && h.name.bytes().all(|b| b.is_ascii_graphic() && b != b':') && !h.value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0);
        if !ok {
            return Err(FetchError::Invalid(format!("header “{}” isn't valid", crate::host::cut(&h.name, 60))));
        }
    }
    if req.body.as_ref().is_some_and(|b| b.len() > MAX_REQUEST_BODY) {
        return Err(FetchError::TooLarge);
    }
    let timeout = req.timeout_ms.map_or(FETCH_TIMEOUT, |ms| Duration::from_millis(u64::from(ms)).clamp(Duration::from_millis(1), MAX_FETCH_TIMEOUT));
    Ok(Outgoing { method, url, headers: req.headers, body: req.body, timeout })
}

/// Called with a request's answer, on any thread.
pub type Done = Box<dyn FnOnce(Result<Response, FetchError>) + Send>;

/// Makes the requests tiles ask for. Called on the tile's worker: start
/// the request and return at once, then call `done` (from any thread)
/// with the answer. `reach` checks each redirect's address.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, request: Outgoing, reach: Reach, done: Done);
}

/// Pitwall's own: ureq with rustls and Mozilla's roots, one thread per
/// request; no cookies, proxy or credentials; redirects (at most
/// [`MAX_REDIRECTS`]) followed only to addresses `reach` allows.
pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn fetch(&self, request: Outgoing, reach: Reach, done: Done) {
        // Kept outside the thread's closure, so a thread that can't start
        // still answers.
        let slot = Arc::new(std::sync::Mutex::new(Some(done)));
        let theirs = slot.clone();
        let spawned = std::thread::Builder::new().name("tile-net".into()).spawn(move || {
            let answer = send(request, &reach);
            if let Some(done) = theirs.lock().ok().and_then(|mut d| d.take()) {
                done(answer);
            }
        });
        if let Err(e) = spawned {
            if let Some(done) = slot.lock().ok().and_then(|mut d| d.take()) {
                done(Err(FetchError::Failed(format!("Pitwall couldn't start the request: {e}"))));
            }
        }
    }
}

fn send(mut req: Outgoing, reach: &Reach) -> Result<Response, FetchError> {
    let agent = ureq::AgentBuilder::new().timeout(req.timeout).redirects(0).user_agent(concat!("pitwall-tile/", env!("CARGO_PKG_VERSION"))).build();
    for _ in 0..=MAX_REDIRECTS {
        let mut r = agent.request_url(&req.method, &req.url);
        for h in &req.headers {
            r = r.set(&h.name, &h.value);
        }
        let resp = match &req.body {
            Some(b) => r.send_bytes(b),
            None => r.call(),
        };
        let resp = match resp {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                return Err(match t.kind() {
                    ureq::ErrorKind::Io if t.to_string().contains("timed out") => FetchError::TimedOut,
                    _ => FetchError::Failed(crate::host::cut(&t.to_string(), 300)),
                })
            }
        };
        if (300..400).contains(&resp.status()) && resp.status() != 304 {
            if let Some(loc) = resp.header("Location") {
                let next = req.url.join(loc).map_err(|e| FetchError::Failed(format!("a redirect to an address that isn't one: {e}")))?;
                if !matches!(next.scheme(), "https" | "http") {
                    return Err(FetchError::Denied(next.as_str().chars().take(200).collect()));
                }
                reach.check(&next).map_err(FetchError::Denied)?;
                // 303, and 301/302 after a POST, go on as GET.
                if resp.status() == 303 || (matches!(resp.status(), 301 | 302) && req.method == "POST") {
                    req.method = "GET".into();
                    req.body = None;
                }
                req.url = next;
                continue;
            }
        }
        return read(resp);
    }
    Err(FetchError::Failed(format!("more than {MAX_REDIRECTS} redirects")))
}

fn read(resp: ureq::Response) -> Result<Response, FetchError> {
    let status = resp.status();
    let url = resp.get_url().to_string();
    let headers: Vec<Header> = resp
        .headers_names()
        .into_iter()
        .filter(|n| !n.eq_ignore_ascii_case("set-cookie"))
        .flat_map(|n| resp.all(&n).into_iter().map(|v| Header { name: n.clone(), value: v.to_string() }).collect::<Vec<_>>())
        .collect();
    if resp.header("Content-Length").and_then(|l| l.trim().parse::<usize>().ok()).is_some_and(|l| l > MAX_RESPONSE_BODY) {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    resp.into_reader().take(MAX_RESPONSE_BODY as u64 + 1).read_to_end(&mut body).map_err(|e| if e.kind() == std::io::ErrorKind::TimedOut { FetchError::TimedOut } else { FetchError::Failed(crate::host::cut(&e.to_string(), 300)) })?;
    if body.len() > MAX_RESPONSE_BODY {
        return Err(FetchError::TooLarge);
    }
    Ok(Response { status, headers, body, url })
}

/// A fetcher that answers from a table, for tests: each URL (exact,
/// query included) with a status and body; anything else is a 404. Every
/// request it got is kept, with its method and body.
#[derive(Default)]
pub struct FakeFetcher {
    answers: std::sync::Mutex<Vec<(String, Result<Response, FetchError>)>>,
    seen: std::sync::Mutex<Vec<Outgoing>>,
    server: std::sync::Mutex<Option<Serve>>,
}

/// A fake server for [`FakeFetcher::serve`]: called with each request no
/// fixed answer matches; it answers with `done`, now or later (a long
/// poll held until the fake has news), from any thread.
pub type Serve = Arc<dyn Fn(Outgoing, Done) + Send + Sync>;

impl FakeFetcher {
    pub fn new() -> Arc<FakeFetcher> {
        Arc::new(FakeFetcher::default())
    }

    /// Answer `url` with `status` and `body` (the newest answer for a URL
    /// wins).
    pub fn respond(&self, url: &str, status: u16, body: impl Into<Vec<u8>>) {
        let r = Response { status, headers: vec![], body: body.into(), url: url.to_string() };
        self.answer(url, Ok(r));
    }

    /// Answer `url` with an error (offline, timed out…).
    pub fn fail(&self, url: &str, error: FetchError) {
        self.answer(url, Err(error));
    }

    fn answer(&self, url: &str, a: Result<Response, FetchError>) {
        if let Ok(mut list) = self.answers.lock() {
            list.retain(|(u, _)| u != url);
            list.push((url.to_string(), a));
        }
    }

    /// The requests it got, oldest first.
    pub fn requests(&self) -> Vec<Outgoing> {
        self.seen.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Answer the requests no fixed answer matches with `server` (a fake
    /// of the service the tile talks to) instead of a 404.
    pub fn serve(&self, server: Serve) {
        if let Ok(mut s) = self.server.lock() {
            *s = Some(server);
        }
    }
}

impl Fetcher for FakeFetcher {
    fn fetch(&self, request: Outgoing, _reach: Reach, done: Done) {
        let url = request.url.to_string();
        if let Ok(mut s) = self.seen.lock() {
            s.push(request.clone());
        }
        let a = self.answers.lock().ok().and_then(|l| l.iter().find(|(u, _)| url::Url::parse(u).is_ok_and(|p| p.as_str() == url)).map(|(_, a)| a.clone()));
        if let Some(a) = a {
            return done(a);
        }
        // Cloned out, so the fake may take its time without the lock.
        let server = self.server.lock().ok().and_then(|s| s.clone());
        match server {
            Some(serve) => serve(request, done),
            None => done(Ok(Response { status: 404, headers: vec![], body: b"not found".to_vec(), url })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::extensions::net::parse_hosts;

    fn reach(hosts: &[&str]) -> Reach {
        Reach { hosts: parse_hosts(hosts).unwrap(), own_server: None }
    }

    fn req(url: &str) -> Request {
        Request { method: "GET".into(), url: url.into(), headers: vec![], body: None, timeout_ms: None }
    }

    #[test]
    fn requests_are_checked_at_the_boundary() {
        let r = reach(&["api.github.com", "*.example.com", "localhost:8790"]);
        assert!(check(req("https://api.github.com/repos"), &r).is_ok());
        assert!(check(req("https://API.GitHub.com./x"), &r).is_err(), "a trailing dot is another name");
        assert!(check(req("https://API.GitHub.com/x"), &r).is_ok(), "parsed to lower case");
        assert!(matches!(check(req("https://evil.com/"), &r), Err(FetchError::Denied(h)) if h == "evil.com"));
        assert!(matches!(check(req("http://api.github.com/"), &r), Err(FetchError::Denied(_))), "plain http only to this computer");
        assert!(check(req("http://localhost:8790/api"), &r).is_ok());
        assert!(matches!(check(req("http://127.0.0.1:8790/api"), &r), Err(FetchError::Denied(_))), "listed as localhost, not the address");
        assert!(matches!(check(req("https://user:pw@api.github.com/"), &r), Err(FetchError::Invalid(_))));
        assert!(matches!(check(req("ftp://api.github.com/"), &r), Err(FetchError::Invalid(_))));
        assert!(matches!(check(req("not a url"), &r), Err(FetchError::Invalid(_))));
        // IDNA: a name in another script is compared in its xn-- form.
        let idn = reach(&["xn--bcher-kva.example"]);
        assert!(check(req("https://bücher.example/"), &idn).is_ok());
        assert!(check(req("https://xn--bcher-kva.example/"), &idn).is_ok());
        let mut h = req("https://api.github.com/");
        h.headers = vec![Header { name: "Cookie".into(), value: "x".into() }];
        assert!(matches!(check(h, &r), Err(FetchError::Invalid(_))));
        let mut m = req("https://api.github.com/");
        m.method = "CONNECT".into();
        assert!(matches!(check(m, &r), Err(FetchError::Invalid(_))));
        let mut big = req("https://api.github.com/");
        big.body = Some(vec![0; MAX_REQUEST_BODY + 1]);
        assert!(matches!(check(big, &r), Err(FetchError::TooLarge)));
        assert!(check(req("https://api.github.com/"), &Reach::default()).is_err(), "nothing granted");
    }

    #[test]
    fn its_own_server_only() {
        let r = Reach { hosts: vec![], own_server: Some("127.0.0.1:8790".parse().unwrap()) };
        assert!(check(req("http://127.0.0.1:8790/api/items"), &r).is_ok());
        assert!(check(req("http://localhost:8790/api/items"), &r).is_ok());
        assert!(check(req("http://127.0.0.1:8791/"), &r).is_err());
        assert!(check(req("https://127.0.0.1:8790/"), &r).is_err(), "it serves http");
        assert!(check(req("http://127.0.0.2:8790/"), &r).is_err());
    }
}
