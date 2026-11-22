//! An extension's adapter process (docs/spec/extensions.md, "Adapter
//! protocol"): started in the extension's folder with its stdin, stdout
//! and stderr piped, talked to in JSON lines on threads of its own (never
//! on the UI thread). What it says arrives as [`Event`]s on a channel the
//! hub reads; a flood, a line over the limit or its end stops it with a
//! reason in plain words.

use std::collections::VecDeque;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use pitwall_core::extensions::protocol::{clean, parse_line, read_line, FromAdapter, LineGuard, ToAdapter};

/// What an adapter did.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Msg(FromAdapter),
    /// It stopped, and why (plain words).
    Stopped(String),
}

/// How long a stopped adapter has to exit after its stdin closes.
const GRACE: Duration = Duration::from_secs(2);
/// How much of its stderr is kept.
const STDERR_KEEP: usize = 4096;

type Shared = Arc<Mutex<Option<Child>>>;

/// The way to an adapter: lines to it, and stopping it.
pub struct Link {
    lines: Option<mpsc::Sender<String>>,
    child: Option<Shared>,
}

impl Link {
    /// Send `m` (false: it is gone).
    pub fn send(&self, m: &ToAdapter) -> bool {
        self.lines.as_ref().is_some_and(|l| l.send(m.line()).is_ok())
    }

    /// Close its stdin; kill it if it hasn't exited after [`GRACE`].
    pub fn stop(mut self) {
        self.lines = None;
        if let Some(child) = self.child.take() {
            std::thread::spawn(move || {
                let end = Instant::now() + GRACE;
                while Instant::now() < end {
                    if exited(&child) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                kill(&child);
            });
        }
    }

    /// Kill it now (Pitwall quits).
    pub fn stop_now(mut self) {
        self.lines = None;
        if let Some(child) = self.child.take() {
            kill(&child);
        }
    }
}

fn exited(child: &Shared) -> bool {
    let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
    match c.as_mut() {
        None => true,
        Some(ch) => !matches!(ch.try_wait(), Ok(None)),
    }
}

fn kill(child: &Shared) {
    if let Some(mut c) = child.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// The program `command[0]` names: a path inside `dir` when it has a
/// slash, else found on `path` (the login shell's PATH).
pub fn resolve(dir: &Path, program: &str, path: Option<&str>) -> Result<PathBuf, String> {
    if program.contains('/') || program.contains('\\') {
        let p = dir.join(program);
        let (Ok(full), Ok(base)) = (p.canonicalize(), dir.canonicalize()) else {
            return Err(format!("{program} isn't in the extension's folder"));
        };
        if !full.starts_with(&base) {
            return Err(format!("{program} isn't inside the extension's folder"));
        }
        return Ok(full);
    }
    let path = path.map(String::from).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    for d in std::env::split_paths(&path) {
        for e in exts {
            let c = d.join(format!("{program}{e}"));
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    Err(format!("“{program}” isn't installed or isn't on your PATH: install it, then restart the extension"))
}

/// Start `command` in `dir` and send it `hello` first.
pub fn spawn(dir: &Path, command: &[String], hello: &ToAdapter) -> Result<(Link, UnboundedReceiver<Event>), String> {
    let program = resolve(dir, &command[0], pitwall_core::shell::spawn_path().as_deref())?;
    let mut cmd = crate::platform::background_command(&program);
    cmd.args(&command[1..]).current_dir(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(p) = pitwall_core::shell::spawn_path() {
        cmd.env("PATH", p);
    }
    let mut child = cmd.spawn().map_err(|e| format!("It couldn't start: {e}"))?;
    let (Some(mut stdin), Some(stdout), Some(mut stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err("It couldn't start: no pipes".into());
    };
    let child: Shared = Arc::new(Mutex::new(Some(child)));
    let (tx, rx) = unbounded();

    // Lines to it, in order, on a thread that may block.
    let (lines, to) = mpsc::channel::<String>();
    let _ = lines.send(hello.line());
    std::thread::spawn(move || {
        while let Ok(l) = to.recv() {
            if stdin.write_all(l.as_bytes()).and_then(|_| stdin.write_all(b"\n")).and_then(|_| stdin.flush()).is_err() {
                return;
            }
        }
    });

    // Its stderr's tail.
    let tail: Arc<Mutex<VecDeque<u8>>> = Arc::default();
    let t = tail.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                return;
            }
            let mut q = t.lock().unwrap_or_else(|e| e.into_inner());
            q.extend(&buf[..n]);
            while q.len() > STDERR_KEEP {
                q.pop_front();
            }
        }
    });

    let c = child.clone();
    std::thread::spawn(move || read_loop(BufReader::new(stdout), tx, c, tail));
    Ok((Link { lines: Some(lines), child: Some(child) }, rx))
}

/// Read its lines until it ends, floods or sends one too long.
fn read_loop(mut out: impl std::io::BufRead, tx: UnboundedSender<Event>, child: Shared, tail: Arc<Mutex<VecDeque<u8>>>) {
    let start = Instant::now();
    let mut guard = LineGuard::default();
    let mut buf = Vec::new();
    let why = loop {
        match read_line(&mut out, &mut buf) {
            Ok(true) => {}
            Ok(false) => break None,
            Err(e) => break Some(format!("It was stopped: it sent {e}.")),
        }
        if let Err(e) = guard.line(start.elapsed().as_millis() as u64) {
            break Some(format!("It was stopped: {e}."));
        }
        if let Ok(msg) = parse_line(&String::from_utf8_lossy(&buf)) {
            if msg != FromAdapter::Unknown && tx.unbounded_send(Event::Msg(msg)).is_err() {
                return;
            }
        }
    };
    let why = match why {
        Some(w) => {
            kill(&child);
            w
        }
        None => {
            // Its output ended: give it a moment to exit, then say how.
            let end = Instant::now() + GRACE;
            let mut code = None;
            while Instant::now() < end {
                let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
                match c.as_mut().map(|ch| ch.try_wait()) {
                    Some(Ok(Some(status))) => {
                        code = Some(status.code());
                        break;
                    }
                    Some(Ok(None)) => {}
                    _ => break,
                }
                drop(c);
                std::thread::sleep(Duration::from_millis(50));
            }
            kill(&child);
            match code {
                Some(Some(0)) => "It exited.".to_string(),
                Some(Some(n)) => format!("It exited with code {n}."),
                _ => "It stopped.".to_string(),
            }
        }
    };
    let err: Vec<u8> = tail.lock().unwrap_or_else(|e| e.into_inner()).iter().copied().collect();
    let err = clean(String::from_utf8_lossy(&err).trim(), 600, true);
    let why = if err.is_empty() { why } else { format!("{why} It said: {err}") };
    let _ = tx.unbounded_send(Event::Stopped(why));
}

/// Tests: a link whose other end the test holds (no process).
#[cfg(test)]
pub struct TestEnd {
    pub lines: mpsc::Receiver<String>,
    pub events: UnboundedSender<Event>,
}

#[cfg(test)]
impl TestEnd {
    /// The lines Pitwall sent so far, as JSON.
    pub fn sent(&self) -> Vec<serde_json::Value> {
        self.lines.try_iter().filter_map(|l| serde_json::from_str(&l).ok()).collect()
    }

    /// The adapter says `line` (parsed as Pitwall parses it).
    pub fn say(&self, line: serde_json::Value) {
        if let Ok(m) = parse_line(&line.to_string()) {
            let _ = self.events.unbounded_send(Event::Msg(m));
        }
    }
}

#[cfg(test)]
pub fn pair(hello: &ToAdapter) -> (Link, UnboundedReceiver<Event>, TestEnd) {
    let (lines, to) = mpsc::channel();
    let _ = lines.send(hello.line());
    let (tx, rx) = unbounded();
    (Link { lines: Some(lines), child: None }, rx, TestEnd { lines: to, events: tx })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn hello() -> ToAdapter {
        ToAdapter::Hello { pitwall: "test".into(), settings: Default::default(), secrets: Default::default() }
    }

    fn script(dir: &Path, body: &str) -> Vec<String> {
        std::fs::write(dir.join("a.sh"), body).unwrap();
        vec!["sh".into(), "a.sh".into()]
    }

    fn next(rx: &mut UnboundedReceiver<Event>) -> Event {
        futures::executor::block_on(async { tokio_free_timeout(rx.next()).await }).expect("an event")
    }

    /// Waits up to 10 s without a runtime of its own.
    async fn tokio_free_timeout<F: std::future::Future<Output = Option<Event>>>(f: F) -> Option<Event> {
        let (tx, rx) = futures::channel::oneshot::channel::<()>();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(10));
            let _ = tx.send(());
        });
        futures::pin_mut!(f);
        match futures::future::select(f, rx).await {
            futures::future::Either::Left((v, _)) => v,
            futures::future::Either::Right(_) => None,
        }
    }

    /// A real process: it gets hello first, its lines arrive, its end and
    /// its stderr are told.
    #[test]
    fn a_process_answers_and_its_end_is_told() {
        let t = pitwall_core::testing::TempDir::new("adapter-proc");
        let cmd = script(t.path(), "read line\ncase \"$line\" in *'\"hello\"'*) echo '{\"type\":\"hello\",\"protocol\":1}';; esac\nread line\necho \"$line\" | grep -q refresh && echo '{\"type\":\"status\",\"state\":\"ready\"}'\necho 'bye now' >&2\nexit 3\n");
        let (link, mut rx) = spawn(t.path(), &cmd, &hello()).unwrap();
        assert_eq!(next(&mut rx), Event::Msg(FromAdapter::Hello { protocol: 1 }));
        assert!(link.send(&ToAdapter::Refresh));
        assert!(matches!(next(&mut rx), Event::Msg(FromAdapter::Status { .. })));
        match next(&mut rx) {
            Event::Stopped(why) => assert!(why.contains("code 3") && why.contains("bye now"), "{why}"),
            e => panic!("{e:?}"),
        }
        link.stop();
    }

    #[test]
    fn a_flood_is_stopped() {
        let t = pitwall_core::testing::TempDir::new("adapter-flood");
        let cmd = script(t.path(), "while :; do echo '{\"type\":\"status\",\"state\":\"ready\"}'; done\n");
        let (link, mut rx) = spawn(t.path(), &cmd, &hello()).unwrap();
        let why = loop {
            if let Event::Stopped(w) = next(&mut rx) {
                break w;
            }
        };
        assert!(why.contains("more than 200 lines"), "{why}");
        link.stop_now();
    }

    /// The extensions shipped in the repo.
    fn shipped(id: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions").join(id)
    }

    fn node() -> Option<PathBuf> {
        let found = resolve(Path::new("."), "node", pitwall_core::shell::spawn_path().as_deref()).ok();
        if found.is_none() {
            eprintln!("node isn't installed: the real adapters aren't checked");
        }
        found
    }

    /// The next message matching `want` (others skipped).
    fn until(rx: &mut UnboundedReceiver<Event>, what: &str, want: impl Fn(&FromAdapter) -> bool) -> FromAdapter {
        loop {
            match next(rx) {
                Event::Msg(m) if want(&m) => return m,
                Event::Msg(_) => {}
                Event::Stopped(why) => panic!("stopped waiting for {what}: {why}"),
            }
        }
    }

    fn hello_with(settings: serde_json::Value, secrets: serde_json::Value) -> ToAdapter {
        let map = |v: serde_json::Value| v.as_object().cloned().unwrap_or_default();
        ToAdapter::Hello { pitwall: "test".into(), settings: map(settings), secrets: map(secrets) }
    }

    /// The fake adapter Pitwall's tests and the docs point to speaks what
    /// Pitwall reads: list, counts, a blocking ask, an answer round trip.
    #[test]
    fn the_demo_adapter_agrees_with_pitwall() {
        if node().is_none() {
            return;
        }
        let (link, mut rx) = spawn(&shipped("demo-inbox"), &["node".into(), "adapter.mjs".into()], &hello()).unwrap();
        assert_eq!(next(&mut rx), Event::Msg(FromAdapter::Hello { protocol: 1 }));
        let FromAdapter::Threads(list) = until(&mut rx, "threads", |m| matches!(m, FromAdapter::Threads(_))) else { unreachable!() };
        assert!(list.len() >= 4 && list.iter().any(|t| t.blocking && t.open_asks > 0 && t.unread > 0), "{list:?}");
        let FromAdapter::Counts(c) = until(&mut rx, "counts", |m| matches!(m, FromAdapter::Counts(_))) else { unreachable!() };
        assert!(c.blocking >= 1 && c.open_asks >= 2, "{c:?}");
        assert!(link.send(&ToAdapter::Open { thread: "demo/1".into() }));
        let FromAdapter::Thread(d) = until(&mut rx, "the thread", |m| matches!(m, FromAdapter::Thread(_))) else { unreachable!() };
        let ask = &d.messages[0].asks[0];
        assert!(ask.open && ask.blocking && ask.suggestions.len() == 3);
        assert!(d.messages[0].detail.contains("| Redis |"), "Markdown detail");
        assert!(link.send(&ToAdapter::Answer { req: 7, thread: "demo/1".into(), ask: 1, picks: vec![1], text: None, declined: false }));
        assert_eq!(until(&mut rx, "the result", |m| matches!(m, FromAdapter::Result { .. })), FromAdapter::Result { req: 7, ok: true, message: String::new() });
        let FromAdapter::Thread(d) = until(&mut rx, "the answered thread", |m| matches!(m, FromAdapter::Thread(_))) else { unreachable!() };
        assert!(!d.messages[0].asks[0].open);
        assert_eq!(d.messages.last().unwrap().answers[0].labels, ["SQLite"]);
        link.stop();
    }

    /// The real Agention adapter, against its fake server, speaks what
    /// Pitwall reads (skipped without Node).
    #[test]
    fn the_agention_adapter_agrees_with_pitwall() {
        let Some(node) = node() else { return };
        let dir = shipped("agention");
        let mut server = std::process::Command::new(&node)
            .arg("test/serve-fake.mjs")
            .current_dir(dir.join("adapter"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut first = String::new();
        std::io::BufRead::read_line(&mut BufReader::new(server.stdout.take().unwrap()), &mut first).unwrap();
        let fake: serde_json::Value = serde_json::from_str(&first).unwrap();
        let thread = fake["thread"].as_str().unwrap().to_string();
        let hello = hello_with(serde_json::json!({"server": fake["url"]}), serde_json::json!({"apiKey": fake["key"]}));
        let (link, mut rx) = spawn(&dir, &["node".into(), "adapter/main.mjs".into()], &hello).unwrap();
        assert_eq!(next(&mut rx), Event::Msg(FromAdapter::Hello { protocol: 1 }));
        until(&mut rx, "ready", |m| matches!(m, FromAdapter::Status { state: pitwall_core::extensions::protocol::ConnState::Ready, .. }));
        let FromAdapter::Threads(list) = until(&mut rx, "threads", |m| matches!(m, FromAdapter::Threads(_))) else { unreachable!() };
        let t = list.iter().find(|t| t.id == thread).expect("the fake's thread");
        assert!(t.blocking && t.open_asks == 1 && t.unread >= 1 && t.from.is_some(), "{t:?}");
        let FromAdapter::Counts(c) = until(&mut rx, "counts", |m| matches!(m, FromAdapter::Counts(_))) else { unreachable!() };
        assert_eq!((c.open_asks, c.blocking), (1, 1));
        assert!(link.send(&ToAdapter::Open { thread: thread.clone() }));
        let FromAdapter::Thread(d) = until(&mut rx, "the thread", |m| matches!(m, FromAdapter::Thread(_))) else { unreachable!() };
        let ask = &d.messages[0].asks[0];
        assert!(ask.open && ask.blocking && ask.suggestions[1].label == "SQLite");
        assert!(link.send(&ToAdapter::Answer { req: 3, thread: thread.clone(), ask: ask.number, picks: vec![1], text: Some("in the data dir".into()), declined: false }));
        assert_eq!(until(&mut rx, "the result", |m| matches!(m, FromAdapter::Result { .. })), FromAdapter::Result { req: 3, ok: true, message: String::new() });
        let FromAdapter::Thread(d) = until(&mut rx, "the answered thread", |m| matches!(m, FromAdapter::Thread(d) if d.messages.len() > 1)) else { unreachable!() };
        let answered = d.messages.iter().flat_map(|m| &m.answers).next().expect("a carried answer");
        assert_eq!((answered.labels.as_slice(), answered.text.as_str()), (&["SQLite".to_string()][..], "in the data dir"));
        link.stop();
        drop(server.stdin.take());
        let _ = server.wait();
    }

    #[test]
    fn programs_are_found_and_kept_in_their_folder() {
        let t = pitwall_core::testing::TempDir::new("adapter-resolve");
        std::fs::write(t.path().join("run.sh"), "").unwrap();
        assert!(resolve(t.path(), "./run.sh", None).is_ok());
        assert!(resolve(t.path(), "../outside.sh", None).is_err());
        assert!(resolve(t.path(), "sh", Some("/bin:/usr/bin")).is_ok());
        assert!(resolve(t.path(), "no-such-program-here", Some("/bin")).unwrap_err().contains("isn't installed"));
    }
}
