//! A stand-in npm registry for tests (extensions from an npm package): a
//! server on this machine's loopback address that serves the metadata
//! documents and tarballs a test publishes, under the https address the
//! manifests name (never contacted: requests are routed to the loopback
//! one, `extensions::npm::routed`). And tarballs made by hand, with what a
//! real one can hold and what Pitwall must refuse (links, `..`, absolute
//! paths, devices).

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// One entry of a tarball made by [`tgz`].
#[derive(Debug, Clone)]
pub struct TarItem {
    pub name: String,
    /// The tar type: `b'0'` a file, `b'5'` a folder, `b'1'` a hard link,
    /// `b'2'` a symbolic link, `b'3'`/`b'4'` a device, `b'6'` a pipe.
    pub kind: u8,
    pub data: Vec<u8>,
    pub mode: u32,
    /// A link's target.
    pub link: String,
}

impl TarItem {
    /// A plain file.
    pub fn file(name: &str, data: impl Into<Vec<u8>>) -> TarItem {
        TarItem { name: name.into(), kind: b'0', data: data.into(), mode: 0o644, link: String::new() }
    }

    /// A file marked executable.
    pub fn exe(name: &str, data: impl Into<Vec<u8>>) -> TarItem {
        TarItem { mode: 0o755, ..TarItem::file(name, data) }
    }

    /// A folder.
    pub fn dir(name: &str) -> TarItem {
        TarItem { name: name.into(), kind: b'5', data: Vec::new(), mode: 0o755, link: String::new() }
    }

    /// A link of type `kind` (`b'1'` hard, `b'2'` symbolic) to `target`.
    pub fn link(kind: u8, name: &str, target: &str) -> TarItem {
        TarItem { name: name.into(), kind, data: Vec::new(), mode: 0o777, link: target.into() }
    }

    /// An entry of type `kind` with no data (a device, a pipe).
    pub fn special(kind: u8, name: &str) -> TarItem {
        TarItem { name: name.into(), kind, data: Vec::new(), mode: 0o644, link: String::new() }
    }
}

/// A ustar header for an entry.
fn header(name: &str, kind: u8, size: u64, mode: u32, link: &str) -> [u8; 512] {
    let mut h = [0u8; 512];
    let put = |h: &mut [u8; 512], at: usize, len: usize, s: &[u8]| h[at..at + s.len().min(len)].copy_from_slice(&s[..s.len().min(len)]);
    put(&mut h, 0, 100, name.as_bytes());
    put(&mut h, 100, 8, format!("{mode:07o}\0").as_bytes());
    put(&mut h, 108, 8, b"0000000\0");
    put(&mut h, 116, 8, b"0000000\0");
    put(&mut h, 124, 12, format!("{size:011o}\0").as_bytes());
    put(&mut h, 136, 12, b"00000000000\0");
    h[156] = kind;
    put(&mut h, 157, 100, link.as_bytes());
    put(&mut h, 257, 6, b"ustar\0");
    put(&mut h, 263, 2, b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u64 = h.iter().map(|b| *b as u64).sum();
    put(&mut h, 148, 8, format!("{sum:06o}\0 ").as_bytes());
    h
}

/// The bytes of an uncompressed tarball of `items` (a name longer than a
/// header holds gets a GNU long-name entry first).
pub fn tar(items: &[TarItem]) -> Vec<u8> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<u8>, data: &[u8]| {
        out.extend_from_slice(data);
        out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
    };
    for i in items {
        if i.name.len() > 100 {
            let long = format!("{}\0", i.name);
            out.extend_from_slice(&header("././@LongLink", b'L', long.len() as u64, 0o644, ""));
            push(&mut out, long.as_bytes());
        }
        out.extend_from_slice(&header(&i.name, i.kind, i.data.len() as u64, i.mode, &i.link));
        push(&mut out, &i.data);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// Bytes gzipped.
pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes).expect("gzip");
    e.finish().expect("gzip")
}

/// A gzipped tarball (`.tgz`) of `items`.
pub fn tgz(items: &[TarItem]) -> Vec<u8> {
    gzip(&tar(items))
}

/// The manifest of extension `id` at `version` published as npm package
/// `package` of `registry`: an adapter, a source naming the package, and
/// `actions` (a JSON array).
pub fn npm_manifest(package: &str, registry: &str, id: &str, version: &str, actions: &str) -> String {
    format!(
        r#"{{"id": "{id}", "name": "{}", "version": "{version}", "adapter": {{"command": ["node", "adapter/main.mjs"]}}, "actions": {actions}, "source": {{"npm": "{package}", "registry": "{registry}"}}}}"#,
        id.to_uppercase()
    )
}

/// The tarball of extension `id` at `version` as npm package `package` of
/// `registry` ([`npm_manifest`]): `package/` holding `package.json`,
/// `extension.json`, `adapter/main.mjs` (holding `main`) and an executable
/// `bin/run`.
pub fn npm_extension(package: &str, registry: &str, id: &str, version: &str, main: &str, actions: &str) -> Vec<u8> {
    tgz(&[
        TarItem::file("package/package.json", format!(r#"{{"name": "{package}", "version": "{version}"}}"#)),
        TarItem::file("package/extension.json", npm_manifest(package, registry, id, version, actions)),
        TarItem::dir("package/adapter/"),
        TarItem::file("package/adapter/main.mjs", main),
        TarItem::exe("package/bin/run", "#!/bin/sh\n"),
    ])
}

#[derive(Default)]
struct State {
    /// Metadata documents by package.
    docs: BTreeMap<String, Value>,
    /// Anything else served, by path (tarballs; raw documents).
    files: HashMap<String, Vec<u8>>,
    /// Each request's head, as received.
    requests: Vec<String>,
}

/// A registry on this machine's loopback address standing in for
/// `registry` (an https address the manifests and metadata name, never
/// contacted). Stops when dropped.
pub struct NpmRegistry {
    /// The address it stands in for.
    pub registry: String,
    /// Where it is served: `http://127.0.0.1:<port>`.
    pub url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
}

impl NpmRegistry {
    /// A registry with nothing published, standing in for `registry`.
    pub fn start(registry: &str) -> NpmRegistry {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback port");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let state: Arc<Mutex<State>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (st, halt) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(c) = conn {
                    let st = st.clone();
                    std::thread::spawn(move || serve(c, &st));
                }
            }
        });
        NpmRegistry { registry: registry.trim_end_matches('/').to_string(), url, state, stop }
    }

    /// The route from the registry's address to this server, for
    /// `extensions::npm::routed`.
    pub fn via(&self) -> Vec<(String, String)> {
        vec![(self.registry.clone(), self.url.clone())]
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Where version `version` of `package`'s tarball is (the registry's
    /// address, as npm's own names it).
    pub fn tarball_url(&self, package: &str, version: &str) -> String {
        let base = package.rsplit('/').next().unwrap_or(package);
        format!("{}/{package}/-/{base}-{version}.tgz", self.registry)
    }

    /// Publish `tgz` as version `version` of `package` (replacing one of
    /// that version: what npm's registry never allows, and Pitwall must
    /// notice): the metadata lists it with the tarball's sha512 integrity.
    /// Returns that integrity.
    pub fn publish(&self, package: &str, version: &str, tgz: &[u8]) -> String {
        let integrity = crate::extensions::npm::integrity_of(tgz);
        self.publish_with(package, version, tgz, Some(&integrity));
        integrity
    }

    /// Publish `tgz` as version `version` of `package` with `integrity` in
    /// the metadata as it is (`None`: only a sha1 `shasum`, as very old
    /// packages have).
    pub fn publish_with(&self, package: &str, version: &str, tgz: &[u8], integrity: Option<&str>) {
        let url = self.tarball_url(package, version);
        let path = url.strip_prefix(&self.registry).unwrap_or(&url).to_string();
        let mut st = self.lock();
        st.files.insert(path, tgz.to_vec());
        let doc = st.docs.entry(package.to_string()).or_insert_with(|| json!({"name": package, "dist-tags": {}, "versions": {}}));
        let mut dist = json!({"tarball": url, "shasum": "0000000000000000000000000000000000000000"});
        if let Some(i) = integrity {
            dist["integrity"] = json!(i);
        }
        doc["versions"][version] = json!({"name": package, "version": version, "dist": dist});
        doc["dist-tags"]["latest"] = json!(version);
    }

    /// Mark version `version` of `package` deprecated.
    pub fn deprecate(&self, package: &str, version: &str) {
        let mut st = self.lock();
        if let Some(v) = st.docs.get_mut(package).and_then(|d| d["versions"].get_mut(version)) {
            v["deprecated"] = json!("Use a newer version");
        }
    }

    /// Serve `bytes` at `path` (of the registry's address) as they are,
    /// over any metadata document there.
    pub fn serve_raw(&self, path: &str, bytes: Vec<u8>) {
        let mut st = self.lock();
        st.files.insert(path.to_string(), bytes);
    }

    /// The heads of the requests it got so far.
    pub fn requests(&self) -> Vec<String> {
        self.lock().requests.clone()
    }
}

impl Drop for NpmRegistry {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accepting thread so it sees the stop.
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}

/// An [`crate::extensions::npm::Http`] for this crate's tests: plain
/// HTTP/1.1 to a loopback address only (an [`NpmRegistry`]), with the
/// limit and wording the app's https client has. Anything else fails, so
/// a test never reaches a real host.
pub struct LoopbackHttp;

impl crate::extensions::npm::Http for LoopbackHttp {
    fn get(&self, url: &str, accept: &str, limit: u64, connect: Duration, timeout: Duration) -> Result<Vec<u8>, crate::extensions::npm::GetError> {
        use crate::extensions::npm::{status_text, GetError, OFFLINE};
        let rest = url.strip_prefix("http://127.0.0.1:").ok_or_else(|| GetError::Failed(format!("not a loopback address: {url}")))?;
        let (port, path) = rest.split_once('/').map(|(p, r)| (p, format!("/{r}"))).unwrap_or((rest, "/".into()));
        let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().map_err(|_| GetError::Failed(OFFLINE.into()))?;
        let mut c = TcpStream::connect_timeout(&addr, connect).map_err(|_| GetError::Failed(OFFLINE.into()))?;
        let _ = c.set_read_timeout(Some(timeout));
        write!(c, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: {accept}\r\nUser-Agent: pitwall-test\r\nConnection: close\r\n\r\n").map_err(|_| GetError::Failed(OFFLINE.into()))?;
        let mut all = Vec::new();
        c.take(limit + 64 * 1024).read_to_end(&mut all).map_err(|_| GetError::Failed(OFFLINE.into()))?;
        let end = all.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| GetError::Failed(OFFLINE.into()))?;
        let head = String::from_utf8_lossy(&all[..end]).into_owned();
        let code: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        if !(200..300).contains(&code) {
            return Err(GetError::Failed(status_text(code)));
        }
        let length = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<u64>().ok()));
        if length.is_some_and(|l| l > limit) {
            return Err(GetError::TooBig);
        }
        let body = all[end + 4..].to_vec();
        if body.len() as u64 > limit {
            return Err(GetError::TooBig);
        }
        Ok(body)
    }
}

/// Answer one connection: a metadata document, a file, or 404.
fn serve(mut c: TcpStream, st: &Mutex<State>) {
    let _ = c.set_read_timeout(Some(Duration::from_secs(10)));
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match c.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
        if head.len() > 64 * 1024 {
            return;
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    let body = {
        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
        s.requests.push(head.clone());
        s.files.get(&path).cloned().or_else(|| {
            let package = path.trim_start_matches('/').replace("%2f", "/").replace("%2F", "/");
            s.docs.get(&package).map(|d| serde_json::to_vec(d).unwrap_or_default())
        })
    };
    let reply = match body {
        Some(b) => {
            let mut r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()).into_bytes();
            r.extend_from_slice(&b);
            r
        }
        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    };
    let _ = c.write_all(&reply);
    let _ = c.flush();
}
