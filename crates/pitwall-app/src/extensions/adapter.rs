//! An extension's adapter process (docs/spec/extensions.md, "Adapter
//! protocol"): started in the extension's folder in a process group of its
//! own (`pitwall_core::extensions::process`), its stdin, stdout and stderr
//! piped, talked to in JSON lines on threads of its own (never on the UI
//! thread). What it says arrives as [`Event`]s on a channel the hub reads;
//! a flood, a line over the limit or its end stops it with a reason in
//! plain words. Stopping it stops what it started too (its group). Every
//! line both ways, and its stderr, go to its log when it has one
//! (`pitwall_core::extensions::log`: secrets taken out).

use std::collections::VecDeque;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use pitwall_core::extensions::log::{self, Dir};
use pitwall_core::extensions::process::{self, Process};
use pitwall_core::extensions::protocol::{clean, parse_line, read_line, FromAdapter, LineGuard, ToAdapter};

/// Where a program is: the core's lookup (a path in the extension's folder,
/// or on the login shell's PATH).
pub use pitwall_core::extensions::plan::resolve;

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

type Shared = Arc<Mutex<Option<Process>>>;
/// Lines to it: one channel, closed for every sender when it is stopped
/// (its stdin closes then).
type Lines = Arc<Mutex<Option<mpsc::Sender<String>>>>;

/// The way to an adapter: lines to it, and stopping it.
pub struct Link {
    lines: Lines,
    child: Option<Shared>,
    log: Option<log::Shared>,
}

/// Add `text` to `log`, when there is one.
fn note(log: &Option<log::Shared>, dir: Dir, text: &str) {
    if let Some(l) = log {
        l.lock().unwrap_or_else(|e| e.into_inner()).push(dir, text);
    }
}

/// A way to send lines to an adapter from any thread (replies to its
/// calls, events), for as long as it runs.
#[derive(Clone)]
pub struct Sender {
    lines: Lines,
    log: Option<log::Shared>,
}

impl Sender {
    /// Send `m` (false: it is gone).
    pub fn send(&self, m: &ToAdapter) -> bool {
        let line = m.line();
        let sent = self.lines.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|l| l.send(line.clone()).is_ok());
        if sent {
            note(&self.log, Dir::ToAdapter, &line);
        }
        sent
    }
}

impl Link {
    /// Send `m` (false: it is gone).
    pub fn send(&self, m: &ToAdapter) -> bool {
        self.sender().is_some_and(|s| s.send(m))
    }

    /// A sender for other threads.
    pub fn sender(&self) -> Option<Sender> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).is_some().then(|| Sender { lines: self.lines.clone(), log: self.log.clone() })
    }

    /// Its process's id (none in tests, or once stopped): what the socket
    /// tells its callers by.
    pub fn pid(&self) -> Option<u32> {
        let c = self.child.as_ref()?;
        let g = c.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref().map(Process::id)
    }

    /// Close its stdin; once it has exited, or after [`GRACE`], stop its
    /// group (what it started goes with it).
    pub fn stop(mut self) {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(child) = self.child.take() {
            std::thread::spawn(move || {
                let end = Instant::now() + GRACE;
                while Instant::now() < end && !exited(&child) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                kill(&child);
            });
        }
    }

    /// Kill it now (Pitwall quits).
    pub fn stop_now(mut self) {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(child) = self.child.take() {
            kill(&child);
        }
    }
}

/// A link let go closes its stdin, whatever senders are still about.
impl Drop for Link {
    fn drop(&mut self) {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

fn exited(child: &Shared) -> bool {
    let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
    match c.as_mut() {
        None => true,
        Some(ch) => ch.exited().is_some(),
    }
}

/// Stop it and its group (once: it is taken out).
fn kill(child: &Shared) {
    if let Some(mut c) = child.lock().unwrap_or_else(|e| e.into_inner()).take() {
        c.stop();
    }
}

/// Start `command` in `dir` and send it `hello` first; `log`: where its
/// lines go.
pub fn spawn(dir: &Path, command: &[String], hello: &ToAdapter, log: Option<log::Shared>) -> Result<(Link, UnboundedReceiver<Event>), String> {
    let path = pitwall_core::shell::spawn_path();
    let program = resolve(dir, &command[0], path.as_deref())?;
    note(&log, Dir::Note, &format!("Starting {}", pitwall_core::extensions::plan::line(command)));
    let (child, pipes) = process::start(&program, &command[1..], dir, path.as_deref()).map_err(|e| format!("It couldn't start: {e}"))?;
    let (mut stdin, stdout, mut stderr) = (pipes.stdin, pipes.stdout, pipes.stderr);
    let child: Shared = Arc::new(Mutex::new(Some(child)));
    let (tx, rx) = unbounded();

    // Lines to it, in order, on a thread that may block.
    let (lines, to) = mpsc::channel::<String>();
    let hello_line = hello.line();
    note(&log, Dir::ToAdapter, &hello_line);
    let _ = lines.send(hello_line);
    std::thread::spawn(move || {
        while let Ok(l) = to.recv() {
            if stdin.write_all(l.as_bytes()).and_then(|_| stdin.write_all(b"\n")).and_then(|_| stdin.flush()).is_err() {
                return;
            }
        }
    });

    // Its stderr's tail, and its lines in the log.
    let tail: Arc<Mutex<VecDeque<u8>>> = Arc::default();
    let t = tail.clone();
    let err_log = log.clone();
    let (err_done, err_ended) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let _done = err_done;
        let mut buf = [0u8; 1024];
        let mut partial: Vec<u8> = Vec::new();
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            {
                let mut q = t.lock().unwrap_or_else(|e| e.into_inner());
                q.extend(&buf[..n]);
                while q.len() > STDERR_KEEP {
                    q.pop_front();
                }
            }
            if err_log.is_some() {
                partial.extend_from_slice(&buf[..n]);
                while let Some(i) = partial.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = partial.drain(..=i).collect();
                    note(&err_log, Dir::Stderr, &String::from_utf8_lossy(&line));
                }
                if partial.len() > log::MAX_TEXT * 4 {
                    note(&err_log, Dir::Stderr, &String::from_utf8_lossy(&partial));
                    partial.clear();
                }
            }
        }
        if !partial.is_empty() {
            note(&err_log, Dir::Stderr, &String::from_utf8_lossy(&partial));
        }
    });

    let c = child.clone();
    let out_log = log.clone();
    std::thread::spawn(move || read_loop(BufReader::new(stdout), tx, c, (tail, err_ended), out_log));
    Ok((Link { lines: Arc::new(Mutex::new(Some(lines))), child: Some(child), log }, rx))
}

/// Read its lines until it ends, floods or sends one too long.
/// `err` is its stderr's tail and the end of its reading.
fn read_loop(mut out: impl std::io::BufRead, tx: UnboundedSender<Event>, child: Shared, err: (Arc<Mutex<VecDeque<u8>>>, std::sync::mpsc::Receiver<()>), log: Option<log::Shared>) {
    let (tail, err_ended) = err;
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
        let text = String::from_utf8_lossy(&buf);
        note(&log, Dir::FromAdapter, &text);
        if let Ok(msg) = parse_line(&text) {
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
                match c.as_mut().map(|ch| ch.exited()) {
                    Some(Some(status)) => {
                        code = Some(status);
                        break;
                    }
                    Some(None) => {}
                    None => break,
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
    // What it printed last on stderr may still be on its way (a moment, as
    // something it started may keep stderr open).
    let _ = err_ended.recv_timeout(Duration::from_millis(500));
    let err: Vec<u8> = tail.lock().unwrap_or_else(|e| e.into_inner()).iter().copied().collect();
    let err = clean(String::from_utf8_lossy(&err).trim(), 600, true);
    note(&log, Dir::Note, &why);
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
pub fn pair(hello: &ToAdapter, log: Option<log::Shared>) -> (Link, UnboundedReceiver<Event>, TestEnd) {
    let (lines, to) = mpsc::channel();
    note(&log, Dir::ToAdapter, &hello.line());
    let _ = lines.send(hello.line());
    let (tx, rx) = unbounded();
    (Link { lines: Arc::new(Mutex::new(Some(lines))), child: None, log }, rx, TestEnd { lines: to, events: tx })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures::StreamExt;
    use pitwall_core::extensions::protocol::{FromInbox, Hello, ToInbox};

    fn hello() -> ToAdapter {
        ToAdapter::Hello(Hello { pitwall: "test".into(), ..Default::default() })
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
    /// its stderr are told; its log has every line both ways and its
    /// stderr, the hello's secrets taken out.
    #[test]
    fn a_process_answers_and_its_end_is_told() {
        let t = pitwall_core::testing::TempDir::new("adapter-proc");
        let cmd = script(t.path(), "read line\ncase \"$line\" in *'\"hello\"'*) echo '{\"type\":\"hello\",\"protocol\":1}';; esac\nread line\necho \"$line\" | grep -q refresh && echo '{\"type\":\"status\",\"state\":\"ready\"}'\necho 'bye now' >&2\nexit 3\n");
        let log: log::Shared = Arc::default();
        let mut secrets = serde_json::Map::new();
        secrets.insert("apiKey".into(), "hunter22".into());
        let hello = ToAdapter::Hello(Hello { pitwall: "test".into(), secrets, ..Default::default() });
        let (link, mut rx) = spawn(t.path(), &cmd, &hello, Some(log.clone())).unwrap();
        assert_eq!(next(&mut rx), Event::Msg(FromAdapter::Hello { protocol: 1, uses: vec![] }));
        assert!(link.send(&ToAdapter::Inbox(ToInbox::Refresh)));
        assert!(matches!(next(&mut rx), Event::Msg(FromAdapter::Status { .. })));
        match next(&mut rx) {
            Event::Stopped(why) => assert!(why.contains("code 3") && why.contains("bye now"), "{why}"),
            e => panic!("{e:?}"),
        }
        link.stop();
        let lines = log.lock().unwrap().lines();
        let seen: Vec<(Dir, &str)> = lines.iter().map(|l| (l.dir, l.text.as_str())).collect();
        assert_eq!(seen[0], (Dir::Note, "Starting sh a.sh"));
        assert!(seen[1].0 == Dir::ToAdapter && seen[1].1.contains("\"apiKey\":\"[secret]\"") && !seen[1].1.contains("hunter22"), "{seen:?}");
        for want in [(Dir::FromAdapter, "{\"type\":\"hello\",\"protocol\":1}"), (Dir::ToAdapter, "{\"type\":\"refresh\"}"), (Dir::Stderr, "bye now"), (Dir::Note, "It exited with code 3.")] {
            assert!(seen.contains(&want), "{want:?} in {seen:?}");
        }
    }

    #[test]
    fn a_flood_is_stopped() {
        let t = pitwall_core::testing::TempDir::new("adapter-flood");
        let cmd = script(t.path(), "while :; do echo '{\"type\":\"status\",\"state\":\"ready\"}'; done\n");
        let (link, mut rx) = spawn(t.path(), &cmd, &hello(), None).unwrap();
        let why = loop {
            if let Event::Stopped(w) = next(&mut rx) {
                break w;
            }
        };
        assert!(why.contains("more than 200 lines"), "{why}");
        link.stop_now();
    }

    /// Stopping an adapter stops what it started: its process group goes.
    #[test]
    fn what_an_adapter_starts_stops_with_it() {
        let t = pitwall_core::testing::TempDir::new("adapter-group");
        let cmd = script(t.path(), "read line\nsleep 60 &\necho \"{\\\"type\\\":\\\"hello\\\",\\\"protocol\\\":$!}\"\nwait\n");
        let (link, mut rx) = spawn(t.path(), &cmd, &hello(), None).unwrap();
        // The child's pid comes back as the "protocol" (a test only).
        let Event::Msg(FromAdapter::Hello { protocol: pid, .. }) = next(&mut rx) else { panic!("no hello") };
        let alive = |pid: u64| std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        assert!(alive(pid), "its child runs");
        link.stop();
        let end = Instant::now() + Duration::from_secs(8);
        while alive(pid) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(pid), "its child stopped with it");
    }

    /// The extensions shipped in the repo.
    fn shipped(id: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions").join(id)
    }

    fn node() -> Option<std::path::PathBuf> {
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

    /// The fake adapter Pitwall's tests and the docs point to speaks what
    /// Pitwall reads: list, counts, a blocking ask, an answer round trip.
    #[test]
    fn the_demo_adapter_agrees_with_pitwall() {
        if node().is_none() {
            return;
        }
        let (link, mut rx) = spawn(&shipped("demo-inbox"), &["node".into(), "adapter.mjs".into()], &hello(), None).unwrap();
        assert_eq!(next(&mut rx), Event::Msg(FromAdapter::Hello { protocol: 1, uses: vec![] }));
        let FromAdapter::Inbox(FromInbox::Threads(list)) = until(&mut rx, "threads", |m| matches!(m, FromAdapter::Inbox(FromInbox::Threads(_)))) else { unreachable!() };
        assert!(list.len() >= 4 && list.iter().any(|t| t.blocking && t.open_asks > 0 && t.unread > 0), "{list:?}");
        let FromAdapter::Inbox(FromInbox::Counts(c)) = until(&mut rx, "counts", |m| matches!(m, FromAdapter::Inbox(FromInbox::Counts(_)))) else { unreachable!() };
        assert!(c.blocking >= 1 && c.open_asks >= 2, "{c:?}");
        assert!(link.send(&ToAdapter::Inbox(ToInbox::Open { thread: "demo/1".into() })));
        let FromAdapter::Inbox(FromInbox::Thread(d)) = until(&mut rx, "the thread", |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(_)))) else { unreachable!() };
        let ask = &d.messages[0].asks[0];
        assert!(ask.open && ask.blocking && ask.suggestions.len() == 3);
        assert!(d.messages[0].detail.contains("| Redis |"), "Markdown detail");
        assert!(link.send(&ToAdapter::Inbox(ToInbox::Answer { req: 7, thread: "demo/1".into(), ask: 1, picks: vec![1], text: None, declined: false })));
        assert_eq!(until(&mut rx, "the result", |m| matches!(m, FromAdapter::Result { .. })), FromAdapter::Result { req: 7, ok: true, message: String::new() });
        let FromAdapter::Inbox(FromInbox::Thread(d)) = until(&mut rx, "the answered thread", |m| matches!(m, FromAdapter::Inbox(FromInbox::Thread(_)))) else { unreachable!() };
        assert!(!d.messages[0].asks[0].open);
        assert_eq!(d.messages.last().unwrap().answers[0].labels, ["SQLite"]);
        link.stop();
    }
}
