//! One agent's terminal, whatever provider it comes from (architecture.md
//! §2.3): the provider supplies a raw byte pipe ([`TermIo`]); this side keeps
//! everything Pitwall does with the bytes, written once: ring + replay,
//! subscribers, the headless `Screen` (and styled frames of it for the
//! Wall), activity/echo timing, paste sending and the redraw nudge.

mod fanout;
mod frames;

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pitwall_detect::Screen;

use crate::clock::Clock;
use crate::error::{ErrorCode, PwError, Result};
use crate::provider::{ExitInfo, TermIo, TermSize};
use fanout::Fanout;
pub use fanout::{coalesce, OutputSink};
pub use frames::{full_frame, FrameSink};

const RING_CAP: usize = 1024 * 1024;
pub const DEFAULT_ROWS: u16 = TermSize::DEFAULT.rows;
pub const DEFAULT_COLS: u16 = TermSize::DEFAULT.cols;
/// Output this soon after user input is most likely echo, not agent work.
const ECHO_WINDOW_MS: u64 = 500;
const MAX_SUBSCRIBERS: usize = 8;

/// Terminal sizes the engine accepts, as (cols, rows).
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    TermSize::new(cols, rows).pair()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct TermHost {
    io: Box<dyn TermIo>,
    output: Mutex<Fanout>,
    pub screen: Mutex<Screen>,
    writer: Mutex<Box<dyn Write + Send>>,
    /// Serialises paste+Enter sequences so two sends never interleave.
    send_lock: Arc<Mutex<()>>,
    size: Mutex<TermSize>,
    /// Bumped on every chunk of output.
    pub output_seq: AtomicU64,
    /// Last output that wasn't likely echo of user input (mono ms).
    pub last_activity: AtomicU64,
    /// Last user keystroke / resize (mono ms).
    pub last_input: AtomicU64,
    /// The connection ended (the agent exited, or — when `!eof_is_exit` —
    /// only the attachment dropped).
    ended: AtomicBool,
    /// Who wants styled frames of the screen: woken on every change.
    watchers: Mutex<Vec<frames::Watcher>>,
    clock: Arc<dyn Clock>,
}

impl TermHost {
    /// Take over a provider's terminal: replay its history into the ring and
    /// screen, then read live output on a thread. `size` is what the caller
    /// asked for; a terminal that knows its own size (a holder after a
    /// re-attach) wins.
    pub fn new(mut io: Box<dyn TermIo>, size: TermSize, clock: Arc<dyn Clock>) -> Arc<TermHost> {
        let size = io.size().unwrap_or(size);
        let history = io.take_history();
        let reader = io.take_reader();
        let writer = io.take_writer();
        let host = Arc::new(TermHost {
            io,
            output: Mutex::new(Fanout::new(RING_CAP, MAX_SUBSCRIBERS)),
            screen: Mutex::new(Screen::new(size.rows, size.cols)),
            writer: Mutex::new(writer),
            send_lock: Arc::new(Mutex::new(())),
            size: Mutex::new(size),
            output_seq: AtomicU64::new(0),
            last_activity: AtomicU64::new(0),
            last_input: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            watchers: Mutex::new(Vec::new()),
            clock,
        });
        if !history.is_empty() {
            // History is not new activity.
            host.on_output(&history, false);
        }
        let h = host.clone();
        let spawned = std::thread::Builder::new().name("term-read".into()).spawn(move || h.read_loop(reader));
        if spawned.is_err() {
            host.ended.store(true, Ordering::Relaxed);
        }
        host
    }

    fn read_loop(&self, mut reader: Box<dyn Read + Send>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.on_output(&buf[..n], true),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        self.ended.store(true, Ordering::Relaxed);
    }

    fn on_output(&self, chunk: &[u8], live: bool) {
        lock(&self.output).push(chunk);
        lock(&self.screen).feed(chunk);
        self.screen_changed();
        let now = self.clock.mono_ms();
        self.output_seq.fetch_add(1, Ordering::Relaxed);
        if live && now.saturating_sub(self.last_input.load(Ordering::Relaxed)) > ECHO_WINDOW_MS {
            self.last_activity.store(now, Ordering::Relaxed);
        }
    }

    /// Replay buffered output, then stream live output to `sink`.
    /// Returns a subscription id for `detach` (unique in this process, so a
    /// stale id never detaches a newer subscriber).
    pub fn attach(self: &Arc<Self>, sink: OutputSink) -> u64 {
        static NEXT_SUB: AtomicU64 = AtomicU64::new(1);
        let sub_id = NEXT_SUB.fetch_add(1, Ordering::Relaxed);
        if !lock(&self.output).subscribe(sub_id, sink) {
            return sub_id;
        }
        // Nudge full-screen TUIs into repainting so the replay ends clean.
        let s = self.clone();
        std::thread::spawn(move || {
            let size = *lock(&s.size);
            if size.rows < 2 || s.ended() {
                return;
            }
            s.touch_input();
            let _ = s.io.resize(TermSize { rows: size.rows - 1, ..size });
            std::thread::sleep(Duration::from_millis(60));
            s.touch_input();
            let _ = s.io.resize(size);
        });
        sub_id
    }

    pub fn detach(&self, sub_id: u64) {
        lock(&self.output).unsubscribe(sub_id);
    }

    /// The buffered output (what a new subscriber gets first).
    pub fn history(&self) -> Vec<u8> {
        lock(&self.output).history()
    }

    /// The screen as text, and its title (status rules read these).
    pub fn screen_text(&self) -> (String, Option<String>) {
        let screen = lock(&self.screen);
        (screen.text(), screen.title())
    }

    /// Current size as (cols, rows). The attach redraw nudge is not reflected.
    pub fn size(&self) -> (u16, u16) {
        lock(&self.size).pair()
    }

    /// Returns whether the size changed.
    pub fn resize(&self, cols: u16, rows: u16) -> bool {
        let size = TermSize::new(cols, rows);
        {
            let mut cur = lock(&self.size);
            if *cur == size {
                return false;
            }
            *cur = size;
        }
        self.touch_input();
        let _ = self.io.resize(size);
        lock(&self.screen).resize(size.rows, size.cols);
        self.screen_changed();
        true
    }

    /// Stream styled frames of the screen to `sink` (the Wall): the whole
    /// screen first, then the rows that changed, at most one frame per
    /// `gap`. Returns an id for `unwatch_screen`.
    pub fn watch_screen(self: &Arc<Self>, sink: FrameSink, gap: Duration) -> u64 {
        static NEXT_WATCH: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_WATCH.fetch_add(1, Ordering::Relaxed);
        if let Some(w) = frames::spawn(id, Arc::downgrade(self), sink, gap) {
            lock(&self.watchers).push(w);
        }
        id
    }

    pub fn unwatch_screen(&self, id: u64) {
        lock(&self.watchers).retain(|w| w.id != id);
    }

    /// Wake the frame watchers (and drop those whose thread has ended).
    fn screen_changed(&self) {
        let mut w = lock(&self.watchers);
        if !w.is_empty() {
            w.retain(|w| w.wake());
        }
    }

    pub fn touch_input(&self) {
        self.last_input.store(self.clock.mono_ms(), Ordering::Relaxed);
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = lock(&self.writer);
        w.write_all(data)
            .and_then(|_| w.flush())
            .map_err(|_| PwError::new(ErrorCode::NotRunning, "agent is not accepting input"))
    }

    /// Bracketed paste of `text` verbatim, then Enter ~200ms later.
    pub fn send_text(self: &Arc<Self>, text: String) {
        let s = self.clone();
        let send_lock = self.send_lock.clone();
        std::thread::spawn(move || {
            let _guard = lock(&send_lock);
            if s.write(&paste_bytes(&text)).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
            let _ = s.write(b"\r");
        });
    }

    /// The connection ended (see `eof_is_exit`).
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }

    /// Whether the end of this connection means the agent exited.
    pub fn eof_is_exit(&self) -> bool {
        self.io.eof_is_exit()
    }

    /// How it ended, once it has.
    pub fn exit(&self) -> Option<ExitInfo> {
        self.io.try_wait().ok().flatten()
    }

    /// The agent's local process (`ProviderCaps::local_process`).
    pub fn pid(&self) -> Option<u32> {
        self.io.pid()
    }

    /// Close the terminal: for a local agent that ends it (hang-up, then a
    /// kill after `grace`; blocks until it is gone), for a remote one only
    /// this attachment. Only for an explicit stop/restart/remove — never
    /// housekeeping (architecture.md §9, decision 1).
    pub fn close(&self, grace: Duration) {
        if !self.ended() || !self.io.eof_is_exit() {
            self.io.close(grace);
        }
        self.ended.store(true, Ordering::Relaxed);
    }
}

pub fn paste_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeTerm;
    use std::time::Instant;

    fn wait_until(f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !f() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    #[test]
    fn paste_is_verbatim() {
        assert_eq!(paste_bytes("a'b\n"), b"\x1b[200~a'b\n\x1b[201~");
    }

    #[test]
    fn sizes_are_clamped() {
        assert_eq!(clamp_size(0, 5000), (2, 1000));
        assert_eq!(clamp_size(80, 24), (80, 24));
    }

    #[test]
    fn history_is_replayed_but_not_activity() {
        let clock = crate::testing::ManualClock::new(10_000);
        let (term, ctl) = FakeTerm::new(TermSize::new(80, 24), Some(7));
        ctl.set_history(b"before ");
        let host = TermHost::new(Box::new(term), TermSize::DEFAULT, clock.clone());
        assert_eq!(host.size(), (80, 24), "the terminal's own size wins");
        assert_eq!(host.history(), b"before ");
        assert_eq!(host.last_activity.load(Ordering::Relaxed), 0);
        ctl.output(b"live");
        assert!(wait_until(|| host.history() == b"before live"));
        assert_eq!(host.last_activity.load(Ordering::Relaxed), 10_000);
        assert!(host.screen_text().0.contains("before live"));
        assert_eq!(host.pid(), Some(7));
    }

    #[test]
    fn screen_frames_while_watched() {
        let clock = crate::testing::ManualClock::new(1);
        let (term, ctl) = FakeTerm::new(TermSize::new(20, 3), None);
        let host = TermHost::new(Box::new(term), TermSize::new(20, 3), clock);
        let got = Arc::new(Mutex::new(Vec::<pitwall_proto::ScreenFrame>::new()));
        let g = got.clone();
        let sink = Box::new(move |f: &pitwall_proto::ScreenFrame| {
            g.lock().unwrap().push(f.clone());
            true
        });
        let id = host.watch_screen(sink, Duration::from_millis(5));
        assert!(wait_until(|| got.lock().unwrap().len() == 1), "the whole screen first");
        assert!(got.lock().unwrap()[0].full);
        ctl.output(b"hello");
        let has_hello = |f: &pitwall_proto::ScreenFrame| !f.full && f.lines.iter().any(|l| l.1.iter().any(|r| r.0 == "hello"));
        assert!(wait_until(|| got.lock().unwrap().iter().any(has_hello)));
        host.unwatch_screen(id);
        let n = got.lock().unwrap().len();
        ctl.output(b" more");
        assert!(wait_until(|| host.screen_text().0.contains("more")));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(got.lock().unwrap().len(), n, "no frames after unwatch");
        assert!(lock(&host.watchers).is_empty());
    }

    #[test]
    fn input_resize_and_exit() {
        let clock = crate::testing::ManualClock::new(1);
        let (term, ctl) = FakeTerm::new(TermSize::new(80, 24), None);
        let host = TermHost::new(Box::new(term), TermSize::new(80, 24), clock);
        host.write(b"hi").unwrap();
        assert_eq!(ctl.input(), b"hi");
        assert!(host.resize(100, 30));
        assert!(!host.resize(100, 30), "same size: nothing to do");
        assert_eq!(ctl.size(), TermSize::new(100, 30));
        ctl.exit(3);
        assert!(wait_until(|| host.ended()));
        assert_eq!(host.exit().and_then(|e| e.code), Some(3));
    }
}
