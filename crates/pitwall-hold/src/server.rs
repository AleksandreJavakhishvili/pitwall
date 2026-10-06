//! The holder process: detach, bind, start the child in a PTY, serve clients.
//! OS-neutral: everything OS-specific goes through `crate::platform`. Plain
//! threads (one for the PTY, one waiting for the child, one accepting, two
//! per client); no async runtime.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use interprocess::local_socket::{prelude::*, Listener, ListenerOptions, Stream};

use crate::platform::{self, PtyControl, PtySpec, Role};
use crate::proto::{self, Info};

/// Output kept for clients that attach later (same size as the app's ring).
pub const RING_CAP: usize = 1024 * 1024;
/// A client this far behind is dropped rather than letting memory grow.
const CLIENT_OUT_MAX: usize = 32 * 1024 * 1024;
pub const DEFAULT_GRACE: Duration = Duration::from_millis(3000);
const REPLAY_CHUNK: usize = 64 * 1024;
/// After the child exits, how long to wait for its last output to drain.
const DRAIN: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub socket: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub cwd: Option<PathBuf>,
    /// How long to keep answering after the child exits.
    pub grace: Duration,
    pub program: OsString,
    pub args: Vec<OsString>,
}

pub const USAGE: &str = "usage: pitwall-hold --socket <path> [--cols N] [--rows N] [--cwd DIR] [--grace-ms N] -- <program> [args...]";

/// Parse the arguments after argv[0].
pub fn parse_args(args: Vec<OsString>) -> Result<Config, String> {
    let mut it = args.into_iter();
    let (mut socket, mut cwd, mut cols, mut rows, mut grace) = (None, None, 120u16, 40u16, DEFAULT_GRACE);
    let mut command = Vec::new();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        let num = |v: OsString, name: &str| -> Result<u64, String> {
            v.to_str().and_then(|s| s.parse().ok()).ok_or_else(|| format!("{name}: not a number"))
        };
        match a.to_str() {
            Some("--socket") => socket = Some(PathBuf::from(value("--socket")?)),
            Some("--cwd") => cwd = Some(PathBuf::from(value("--cwd")?)),
            Some("--cols") => cols = num(value("--cols")?, "--cols")?.clamp(1, 1000) as u16,
            Some("--rows") => rows = num(value("--rows")?, "--rows")?.clamp(1, 1000) as u16,
            Some("--grace-ms") => grace = Duration::from_millis(num(value("--grace-ms")?, "--grace-ms")?),
            Some("--") => {
                command.extend(it.by_ref());
                break;
            }
            _ => return Err(format!("unexpected argument {:?}\n{USAGE}", a)),
        }
    }
    let socket = socket.ok_or_else(|| format!("--socket is required\n{USAGE}"))?;
    if command.is_empty() {
        return Err(format!("no program given\n{USAGE}"));
    }
    let program = command.remove(0);
    Ok(Config { socket, cols, rows, cwd, grace, program, args: command })
}

/// Launcher entry point: detach a holder, wait for its report, return the
/// launcher's exit code. The holder side never returns from here.
pub fn run(cfg: Config) -> i32 {
    match platform::detach() {
        Err(e) => {
            eprintln!("pitwall-hold: could not detach: {e}");
            1
        }
        Ok(Role::Launcher(mut report)) => {
            let mut text = String::new();
            let _ = report.read_to_string(&mut text);
            match text.strip_prefix("ready ") {
                Some(rest) => {
                    print!("ready {rest}");
                    let _ = io::stdout().flush();
                    0
                }
                None => {
                    let msg = text.trim();
                    eprintln!("pitwall-hold: {}", if msg.is_empty() { "the holder failed to start" } else { msg });
                    1
                }
            }
        }
        Ok(Role::Holder(mut ready)) => {
            let hub = match Hub::start(&cfg) {
                Ok(h) => h,
                Err(e) => {
                    let _ = write!(ready, "{e}");
                    std::process::exit(1);
                }
            };
            let _ = writeln!(ready, "ready {} {}", hub.holder_pid, hub.child_pid);
            drop(ready);
            hub.serve();
            std::process::exit(0);
        }
    }
}

/// Bind the endpoint, taking over a stale one or one whose child has exited,
/// refusing if a live holder with a running child answers there.
pub fn bind(path: &Path) -> io::Result<(Listener, Option<u64>)> {
    platform::prepare_endpoint(path)?;
    match crate::client::connect(path, Duration::from_secs(2)) {
        Ok(c) if c.info.exit.is_none() => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("a holder with a running process already uses {}", path.display()),
            ))
        }
        // Its child is gone; the old holder is only lingering.
        Ok(_) => platform::remove_stale_endpoint(path),
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => platform::remove_stale_endpoint(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{} is in use by something that isn't a compatible holder: {e}", path.display()),
            ))
        }
    }
    let listener = ListenerOptions::new()
        .name(platform::endpoint_name(path)?)
        .reclaim_name(false)
        .create_sync()?;
    platform::secure_endpoint(path)?;
    Ok((listener, platform::endpoint_token(path)))
}

struct Client {
    id: u64,
    out: Out,
    attached: bool,
}

/// The sending side of one connection: frames go to its writer thread.
#[derive(Clone)]
struct Out {
    tx: Sender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
}

impl Out {
    /// False when the client is gone or too far behind.
    fn push(&self, frame: Vec<u8>) -> bool {
        let len = frame.len();
        if self.queued.fetch_add(len, Ordering::SeqCst) + len > CLIENT_OUT_MAX {
            return false;
        }
        self.tx.send(frame).is_ok()
    }
}

struct State {
    ring: VecDeque<u8>,
    clients: Vec<Client>,
    cols: u16,
    rows: u16,
    exit: Option<i32>,
    output_done: bool,
    shutdown: bool,
    linger_until: Option<Instant>,
}

struct Hub {
    st: Mutex<State>,
    cv: Condvar,
    control: PtyControl,
    input: Mutex<Box<dyn Write + Send>>,
    holder_pid: u32,
    child_pid: u32,
    grace: Duration,
    socket: PathBuf,
    token: Option<u64>,
    // Taken by `serve`.
    pending: Mutex<Option<(Listener, Box<dyn Read + Send>)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Hub {
    fn start(cfg: &Config) -> io::Result<Arc<Hub>> {
        let (listener, token) = bind(&cfg.socket)?;
        let pty = match platform::spawn_pty(&PtySpec {
            program: cfg.program.clone(),
            args: cfg.args.clone(),
            cwd: cfg.cwd.clone(),
            cols: cfg.cols,
            rows: cfg.rows,
        }) {
            Ok(p) => p,
            Err(e) => {
                platform::remove_endpoint(&cfg.socket, token);
                return Err(io::Error::new(e.kind(), format!("could not start the terminal: {e}")));
            }
        };
        Ok(Arc::new(Hub {
            st: Mutex::new(State {
                ring: VecDeque::new(),
                clients: Vec::new(),
                cols: cfg.cols,
                rows: cfg.rows,
                exit: None,
                output_done: false,
                shutdown: false,
                linger_until: None,
            }),
            cv: Condvar::new(),
            control: pty.control,
            input: Mutex::new(pty.writer),
            holder_pid: std::process::id(),
            child_pid: pty.pid,
            grace: cfg.grace,
            socket: cfg.socket.clone(),
            token,
            pending: Mutex::new(Some((listener, pty.reader))),
        }))
    }

    fn info(&self, st: &State) -> Info {
        Info { holder_pid: self.holder_pid, child_pid: self.child_pid, cols: st.cols, rows: st.rows, exit: st.exit }
    }

    /// Send to every attached client, dropping the ones that can't keep up.
    fn broadcast(st: &mut State, frame: &[u8]) {
        st.clients.retain(|c| !c.attached || c.out.push(frame.to_vec()));
    }

    fn serve(self: &Arc<Self>) {
        let (listener, reader) = lock(&self.pending).take().expect("serve once");
        let h = self.clone();
        std::thread::spawn(move || h.pump_output(reader));
        let h = self.clone();
        std::thread::spawn(move || h.wait_child());
        let h = self.clone();
        std::thread::spawn(move || h.accept(listener));

        let mut st = lock(&self.st);
        loop {
            if let Some(until) = st.linger_until {
                let now = Instant::now();
                let flushed = st.clients.iter().all(|c| c.out.queued.load(Ordering::SeqCst) == 0);
                if (flushed && (st.shutdown || now >= until)) || now >= until + Duration::from_secs(1) {
                    break;
                }
            }
            st = self.cv.wait_timeout(st, Duration::from_millis(50)).unwrap_or_else(|e| e.into_inner()).0;
        }
        drop(st);
        platform::remove_endpoint(&self.socket, self.token);
    }

    fn pump_output(&self, mut reader: Box<dyn Read + Send>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut st = lock(&self.st);
                    push_ring(&mut st.ring, &buf[..n], RING_CAP);
                    let frame = proto::encode(proto::OUTPUT, &buf[..n]);
                    Self::broadcast(&mut st, &frame);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                // EIO: every process holding the terminal has closed it.
                Err(_) => break,
            }
        }
        lock(&self.st).output_done = true;
        self.cv.notify_all();
    }

    fn wait_child(&self) {
        let code = self.control.wait();
        // Let the last output through before EXIT.
        let deadline = Instant::now() + DRAIN;
        let mut st = lock(&self.st);
        while !st.output_done && Instant::now() < deadline {
            st = self.cv.wait_timeout(st, Duration::from_millis(20)).unwrap_or_else(|e| e.into_inner()).0;
        }
        st.exit = Some(code);
        st.linger_until = Some(Instant::now() + self.grace);
        Self::broadcast(&mut st, &proto::encode(proto::EXIT, &code.to_be_bytes()));
        drop(st);
        self.cv.notify_all();
    }

    fn accept(self: Arc<Self>, listener: Listener) {
        let mut next_id = 0u64;
        loop {
            match listener.accept() {
                Ok(stream) => {
                    next_id += 1;
                    let (h, id) = (self.clone(), next_id);
                    std::thread::spawn(move || h.client(id, stream));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// One connection: a writer thread drains its queue, this thread reads.
    fn client(&self, id: u64, stream: Stream) {
        let (mut recv, mut send) = stream.split();
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let out = Out { tx, queued: Arc::new(AtomicUsize::new(0)) };
        let queued = out.queued.clone();
        std::thread::spawn(move || {
            for frame in rx {
                if send.write_all(&frame).is_err() {
                    break;
                }
                queued.fetch_sub(frame.len(), Ordering::SeqCst);
            }
            // Dropping `send` (and `recv` below) closes the connection.
        });

        let mut greeted = false;
        while let Ok(Some((ty, body))) = proto::read_frame(&mut recv) {
            if !greeted {
                if ty != proto::HELLO {
                    out.push(proto::encode(proto::ERROR, b"expected HELLO"));
                    break;
                }
                greeted = true;
                let info = self.info(&lock(&self.st));
                out.push(proto::welcome(&info));
                let version = body.get(0..2).map(|v| u16::from_be_bytes([v[0], v[1]]));
                if version != Some(proto::PROTOCOL_VERSION) {
                    break;
                }
                continue;
            }
            if !self.handle(id, &out, ty, &body) {
                break;
            }
        }
        lock(&self.st).clients.retain(|c| c.id != id);
    }

    /// Returns false to drop the connection.
    fn handle(&self, id: u64, out: &Out, ty: u8, body: &[u8]) -> bool {
        match ty {
            proto::ATTACH => {
                let mut st = lock(&self.st);
                if st.clients.iter().any(|c| c.id == id) {
                    return true;
                }
                if body.first() == Some(&1) {
                    let (a, b) = st.ring.as_slices();
                    let ring = [a, b].concat();
                    for chunk in ring.chunks(REPLAY_CHUNK) {
                        if !out.push(proto::encode(proto::REPLAY, chunk)) {
                            return false;
                        }
                    }
                }
                if let Some(code) = st.exit {
                    out.push(proto::encode(proto::EXIT, &code.to_be_bytes()));
                }
                st.clients.push(Client { id, out: out.clone(), attached: true });
            }
            proto::INPUT => {
                if lock(&self.st).exit.is_none() {
                    let mut input = lock(&self.input);
                    let _ = input.write_all(body).and_then(|_| input.flush());
                }
            }
            proto::RESIZE => {
                if body.len() >= 4 {
                    let cols = u16::from_be_bytes([body[0], body[1]]).clamp(1, 1000);
                    let rows = u16::from_be_bytes([body[2], body[3]]).clamp(1, 1000);
                    let mut st = lock(&self.st);
                    st.cols = cols;
                    st.rows = rows;
                    if st.exit.is_none() {
                        self.control.resize(cols, rows);
                    }
                }
            }
            proto::STATUS => {
                let mut b = Vec::new();
                self.info(&lock(&self.st)).encode(&mut b);
                return out.push(proto::encode(proto::INFO, &b));
            }
            proto::SHUTDOWN => {
                let grace = body.get(0..4).map_or(0, |g| u32::from_be_bytes([g[0], g[1], g[2], g[3]]));
                let mut st = lock(&self.st);
                st.shutdown = true;
                if st.exit.is_none() {
                    self.control.hangup();
                    let control = self.control.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(grace as u64));
                        // A no-op once the child has been reaped.
                        control.kill();
                    });
                }
                drop(st);
                self.cv.notify_all();
            }
            other => {
                let msg = format!("unknown frame type 0x{other:02x}");
                return out.push(proto::encode(proto::ERROR, msg.as_bytes()));
            }
        }
        true
    }
}

pub fn push_ring(ring: &mut VecDeque<u8>, chunk: &[u8], cap: usize) {
    if chunk.len() >= cap {
        ring.clear();
        ring.extend(&chunk[chunk.len() - cap..]);
        return;
    }
    let overflow = (ring.len() + chunk.len()).saturating_sub(cap);
    ring.drain(..overflow);
    ring.extend(chunk);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<OsString> {
        a.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_arguments() {
        let c = parse_args(args(&["--socket", "/t/a.sock", "--cols", "77", "--rows", "0", "--cwd", "/w", "--grace-ms", "50", "--", "sh", "-c", "echo hi"])).unwrap();
        assert_eq!(c.socket, PathBuf::from("/t/a.sock"));
        assert_eq!((c.cols, c.rows), (77, 1));
        assert_eq!(c.cwd, Some(PathBuf::from("/w")));
        assert_eq!(c.grace, Duration::from_millis(50));
        assert_eq!(c.program, OsString::from("sh"));
        assert_eq!(c.args, args(&["-c", "echo hi"]));

        let d = parse_args(args(&["--socket", "s", "--", "x", "--cols"])).unwrap();
        assert_eq!((d.cols, d.rows, d.grace), (120, 40, DEFAULT_GRACE));
        assert_eq!(d.args, args(&["--cols"]), "args after -- are the program's");

        assert!(parse_args(args(&["--", "sh"])).is_err(), "socket required");
        assert!(parse_args(args(&["--socket", "s"])).is_err(), "program required");
        assert!(parse_args(args(&["--socket", "s", "--cols", "x", "--", "sh"])).is_err());
        assert!(parse_args(args(&["--bogus"])).is_err());
    }

    #[test]
    fn ring_keeps_newest_bytes() {
        let mut ring = VecDeque::new();
        push_ring(&mut ring, b"abcdef", 8);
        push_ring(&mut ring, b"ghij", 8);
        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), b"cdefghij");
        push_ring(&mut ring, b"0123456789", 8);
        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), b"23456789");
    }
}
