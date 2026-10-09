//! One agent's terminal, whatever provider it comes from (architecture.md
//! §2.3): the provider supplies a raw byte pipe ([`TermIo`]); this side keeps
//! everything Pitwall does with the bytes, written once: ring + replay,
//! subscribers, the headless `Screen` (and styled frames of it for the
//! Wall), activity/echo timing, input writing and paste sending, and the
//! redraw nudge.

mod fanout;
mod frames;
mod modes;

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pitwall_detect::Screen;

use crate::clock::Clock;
use crate::error::{ErrorCode, PwError, Result};
use crate::provider::{ExitInfo, TermIo, TermSize};
use fanout::Fanout;
pub use fanout::{coalesce, OutputSink};
pub use frames::{full_frame, FrameSink};
pub use pitwall_detect::ScreenSource;

const RING_CAP: usize = 1024 * 1024;
pub const DEFAULT_ROWS: u16 = TermSize::DEFAULT.rows;
pub const DEFAULT_COLS: u16 = TermSize::DEFAULT.cols;
/// Output this soon after user input is most likely echo, not agent work.
const ECHO_WINDOW_MS: u64 = 500;
const MAX_SUBSCRIBERS: usize = 8;
/// Largest piece of input written at once. A long paste goes to the program
/// in pieces, as terminals write it, each after the previous one was taken.
pub const INPUT_CHUNK: usize = 1024;
/// A cut never falls this close after an ESC: escape sequences (the paste
/// brackets, keys, mouse reports) reach the program whole.
const ESC_SEQ_MAX: usize = 32;
/// Between a sent prompt's paste and its Enter.
const SEND_ENTER_DELAY: Duration = Duration::from_millis(200);

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
    /// What the status rules and the Wall read: a headless [`Screen`], or a
    /// UI's terminal parsing the same output ([`TermHost::share_screen`]).
    /// Locked after `output` when both are.
    screen: Mutex<HostScreen>,
    /// Input for the writer thread, in order: keystrokes, pastes and sent
    /// prompts queue behind each other and never block the caller.
    input: Mutex<Sender<Input>>,
    /// The writer failed (the terminal no longer takes input).
    input_failed: Arc<AtomicBool>,
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
        let (input, input_rx) = mpsc::channel();
        let input_failed = Arc::new(AtomicBool::new(false));
        let failed = input_failed.clone();
        if std::thread::Builder::new().name("term-write".into()).spawn(move || write_loop(writer, input_rx, &failed)).is_err() {
            input_failed.store(true, Ordering::Relaxed);
        }
        let host = Arc::new(TermHost {
            io,
            output: Mutex::new(Fanout::new(RING_CAP, MAX_SUBSCRIBERS)),
            screen: Mutex::new(HostScreen::own(Screen::new(size.rows, size.cols))),
            input: Mutex::new(input),
            input_failed,
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
        // The screen is taken before the ring lets go: a chunk is either in
        // the history a new screen source replays or fed to it, never both.
        let mut out = lock(&self.output);
        out.push(chunk);
        let mut screen = lock(&self.screen);
        drop(out);
        screen.source.feed(chunk);
        let gone = !screen.source.alive();
        drop(screen);
        if gone {
            self.own_screen();
        }
        self.screen_changed();
        let now = self.clock.mono_ms();
        self.output_seq.fetch_add(1, Ordering::Relaxed);
        if live && now.saturating_sub(self.last_input.load(Ordering::Relaxed)) > ECHO_WINDOW_MS {
            self.last_activity.store(now, Ordering::Relaxed);
        }
    }

    /// Let `source` (a UI's terminal emulator) be this agent's screen: it
    /// gets the buffered output, then every chunk as it comes, and the
    /// status rules and the Wall read it instead of a headless screen of
    /// the engine's own (one parser for the agent, not two). The engine
    /// goes back to a screen of its own once `source` is no longer
    /// [`alive`](ScreenSource::alive).
    pub fn share_screen(self: &Arc<Self>, mut source: Box<dyn ScreenSource>) {
        {
            let out = lock(&self.output);
            let mut screen = lock(&self.screen);
            source.feed(&out.history());
            *screen = HostScreen { source, shared: true };
        }
        self.screen_changed();
        self.nudge_redraw();
    }

    /// The shared screen is gone: back to a headless screen of the
    /// engine's own, rebuilt from the buffered output.
    fn own_screen(&self) {
        let out = lock(&self.output);
        let mut screen = lock(&self.screen);
        if !screen.shared || screen.source.alive() {
            return;
        }
        let size = *lock(&self.size);
        let mut own = Screen::new(size.rows, size.cols);
        own.feed(&out.history());
        *screen = HostScreen::own(own);
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
        self.nudge_redraw();
        sub_id
    }

    /// Nudge full-screen TUIs into repainting so a replay ends clean.
    fn nudge_redraw(self: &Arc<Self>) {
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
            // The size now: a view that fitted itself meanwhile (a native
            // terminal sizes itself right after attaching) must not be
            // undone by the old size.
            let now = *lock(&s.size);
            let _ = s.io.resize(now);
        });
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
        (screen.source.text(), screen.source.title())
    }

    /// The screen as styled rows (what the Wall draws).
    pub fn snapshot(&self) -> pitwall_detect::screen::Snapshot {
        lock(&self.screen).source.snapshot()
    }

    /// Whether a UI's terminal is the screen ([`share_screen`](Self::share_screen)).
    pub fn screen_shared(&self) -> bool {
        lock(&self.screen).shared
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
        lock(&self.screen).source.resize(size.rows, size.cols);
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

    /// Queue input for the program; returns at once (the writer thread
    /// writes it in order, in pieces of at most [`INPUT_CHUNK`]).
    pub fn write(&self, data: &[u8]) -> Result<()> {
        self.queue(&[Input::Bytes(data.to_vec())])
    }

    /// Bracketed paste of `text` verbatim, then Enter ~200ms after it was
    /// written. Input queued later (typing) goes after the Enter.
    pub fn send_text(&self, text: String) {
        let _ = self.queue(&[Input::Bytes(paste_bytes(&text)), Input::Pause(SEND_ENTER_DELAY), Input::Bytes(b"\r".to_vec())]);
    }

    fn queue(&self, items: &[Input]) -> Result<()> {
        let not_running = || PwError::new(ErrorCode::NotRunning, "agent is not accepting input");
        if self.input_failed.load(Ordering::Relaxed) {
            return Err(not_running());
        }
        // One lock for the whole sequence: nothing queues in between.
        let tx = lock(&self.input);
        for item in items {
            tx.send(item.clone()).map_err(|_| not_running())?;
        }
        Ok(())
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

/// The agent's screen, and whether it is a UI's (shared) or the engine's.
struct HostScreen {
    source: Box<dyn ScreenSource>,
    shared: bool,
}

impl HostScreen {
    fn own(screen: Screen) -> HostScreen {
        HostScreen { source: Box::new(screen), shared: false }
    }
}

#[derive(Clone)]
enum Input {
    Bytes(Vec<u8>),
    Pause(Duration),
}

/// The writer thread: input in order, a long one in pieces, each written
/// (and flushed) before the next, so a program that reads slowly holds the
/// rest back instead of having it pile up in (or overflow) its terminal's
/// input queue. Ends when the writer fails or the host is dropped.
fn write_loop(mut w: Box<dyn Write + Send>, input: Receiver<Input>, failed: &AtomicBool) {
    for item in input {
        match item {
            Input::Bytes(data) => {
                for piece in input_pieces(&data, INPUT_CHUNK) {
                    if write_piece(&mut w, piece).is_err() {
                        failed.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            }
            Input::Pause(d) => std::thread::sleep(d),
        }
    }
}

/// `write_all` + flush that waits out a full non-blocking writer.
fn write_piece(w: &mut dyn Write, mut buf: &[u8]) -> std::io::Result<()> {
    while !buf.is_empty() {
        match w.write(buf) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(e),
        }
    }
    w.flush()
}

/// `data` in pieces of at most `max` bytes (`max` ≥ 8) that never split a
/// UTF-8 character or a short escape sequence (a paste's `ESC[200~` /
/// `ESC[201~`): a program that sees a lone ESC at the end of one read may
/// take it for the Escape key.
pub fn input_pieces(data: &[u8], max: usize) -> impl Iterator<Item = &[u8]> {
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (piece, more) = rest.split_at(piece_len(rest, max));
        rest = more;
        Some(piece)
    })
}

fn piece_len(data: &[u8], max: usize) -> usize {
    if data.len() <= max {
        return data.len();
    }
    let mut end = max;
    // Cut before an ESC close to the end (complete or not: a cut there is harmless).
    let from = end.saturating_sub(ESC_SEQ_MAX).max(1);
    if let Some(i) = data[from..end].iter().rposition(|&b| b == 0x1b) {
        end = from + i;
    }
    // Not inside a UTF-8 character.
    while end > 1 && data[end] & 0xC0 == 0x80 {
        end -= 1;
    }
    end
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

    /// A UI's terminal standing in as the screen: records what it is fed.
    struct UiScreen {
        fed: Arc<Mutex<Vec<u8>>>,
        screen: Screen,
        alive: Arc<AtomicBool>,
    }

    impl ScreenSource for UiScreen {
        fn feed(&mut self, bytes: &[u8]) {
            self.fed.lock().unwrap().extend_from_slice(bytes);
            self.screen.feed(bytes);
        }
        fn resize(&mut self, _rows: u16, _cols: u16) {}
        fn text(&self) -> String {
            format!("ui: {}", self.screen.text())
        }
        fn title(&self) -> Option<String> {
            Some("ui".into())
        }
        fn snapshot(&self) -> pitwall_detect::screen::Snapshot {
            self.screen.snapshot()
        }
        fn alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
    }

    fn ui_screen() -> (Box<UiScreen>, Arc<Mutex<Vec<u8>>>, Arc<AtomicBool>) {
        let fed = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        (Box::new(UiScreen { fed: fed.clone(), screen: Screen::new(3, 20), alive: alive.clone() }), fed, alive)
    }

    #[test]
    fn a_shared_screen_gets_every_byte_once_and_is_what_rules_read() {
        let clock = crate::testing::ManualClock::new(1);
        let (term, ctl) = FakeTerm::new(TermSize::new(20, 3), None);
        ctl.set_history(b"before ");
        let host = TermHost::new(Box::new(term), TermSize::new(20, 3), clock);
        // Output keeps coming while the screen is handed over.
        let writer = {
            let ctl = ctl.clone();
            std::thread::spawn(move || {
                for i in 0..200 {
                    ctl.output(format!("{i} ").as_bytes());
                }
            })
        };
        let (ui, fed, alive) = ui_screen();
        std::thread::sleep(Duration::from_millis(1));
        host.share_screen(ui);
        writer.join().unwrap();
        ctl.output(b"end");
        assert!(wait_until(|| host.history().ends_with(b"end")));
        assert!(wait_until(|| fed.lock().unwrap().ends_with(b"end")));
        assert_eq!(*fed.lock().unwrap(), host.history(), "the replay, then each chunk once");
        assert!(host.screen_shared());
        let (text, title) = host.screen_text();
        assert!(text.starts_with("ui: ") && text.ends_with("end"), "{text:?}");
        assert_eq!(title.as_deref(), Some("ui"));
        assert_eq!(full_frame(&host.snapshot()).lines.last().map(|l| l.0), Some(2), "the Wall reads it too");

        // The UI's terminal is gone: the engine's own screen, with everything.
        alive.store(false, Ordering::SeqCst);
        ctl.output(b" after");
        assert!(wait_until(|| !host.screen_shared()));
        assert!(wait_until(|| host.screen_text().0.replace('\n', "").ends_with("end after")));
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
        assert!(wait_until(|| ctl.input() == b"hi"));
        assert!(host.resize(100, 30));
        assert!(!host.resize(100, 30), "same size: nothing to do");
        assert_eq!(ctl.size(), TermSize::new(100, 30));
        ctl.exit(3);
        assert!(wait_until(|| host.ended()));
        assert_eq!(host.exit().and_then(|e| e.code), Some(3));
    }

    /// A long, multi-line, non-ASCII text (Georgian, accents, emoji).
    fn long_text(len: usize) -> String {
        let mut s = String::new();
        let mut i = 0;
        while s.len() < len {
            s.push_str(&format!("{i:06} ქართული ტექსტი — résumé ✓ 🙂 tab\there\n"));
            i += 1;
        }
        s
    }

    fn check_pieces(data: &[u8], max: usize) -> Vec<&[u8]> {
        let pieces: Vec<&[u8]> = input_pieces(data, max).collect();
        assert_eq!(pieces.concat(), data, "every byte, in order");
        for p in &pieces {
            assert!(!p.is_empty() && p.len() <= max, "piece of {} bytes", p.len());
            assert!(std::str::from_utf8(p).is_ok(), "a piece splits a character");
            // The only ESCs are the paste brackets: each piece holds them whole.
            for (i, _) in p.iter().enumerate().filter(|(_, &b)| b == 0x1b) {
                let rest = &p[i..];
                assert!(rest.starts_with(b"\x1b[200~") || rest.starts_with(b"\x1b[201~"), "a bracket is split");
            }
        }
        pieces
    }

    #[test]
    fn long_pastes_go_in_pieces_with_the_brackets_whole() {
        let paste = paste_bytes(&long_text(256 * 1024));
        let pieces = check_pieces(&paste, INPUT_CHUNK);
        assert!(pieces.len() > 250);
        assert!(pieces[0].starts_with(b"\x1b[200~"));
        assert!(pieces.last().unwrap().ends_with(b"\x1b[201~"));
        // The end bracket (and a 3-byte character) right where a cut would
        // fall, at every offset.
        for n in INPUT_CHUNK - 20..INPUT_CHUNK + 4 {
            let text = "x".repeat(n - 3) + "ქ";
            let paste = paste_bytes(&text);
            let pieces = check_pieces(&paste, INPUT_CHUNK);
            assert_eq!(pieces.len(), if paste.len() <= INPUT_CHUNK { 1 } else { 2 });
        }
        // Short input goes as it is.
        assert_eq!(input_pieces(b"\x1b[A", INPUT_CHUNK).collect::<Vec<_>>(), vec![b"\x1b[A"]);
        assert_eq!(input_pieces(b"", INPUT_CHUNK).count(), 0);
    }

    /// A program that reads slowly: records each write, taking a moment.
    struct SlowWriter(Arc<Mutex<Vec<Vec<u8>>>>);

    impl Write for SlowWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_micros(200));
            self.0.lock().unwrap().push(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn input_is_written_in_order_without_blocking_the_caller() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let failed = Arc::new(AtomicBool::new(false));
        let (w, f) = (SlowWriter(writes.clone()), failed.clone());
        std::thread::spawn(move || write_loop(Box::new(w), rx, &f));
        let paste = paste_bytes(&long_text(1024 * 1024));
        let host_like = |items: Vec<Input>| items.into_iter().for_each(|i| tx.send(i).unwrap());
        let t0 = Instant::now();
        host_like(vec![Input::Bytes(paste.clone()), Input::Pause(Duration::from_millis(1)), Input::Bytes(b"\r".to_vec())]);
        host_like(vec![Input::Bytes(b"typed".to_vec())]);
        assert!(t0.elapsed() < Duration::from_millis(200), "queued, not written, by the caller");
        let want = [paste.clone(), b"\r".to_vec(), b"typed".to_vec()].concat();
        assert!(wait_until(|| writes.lock().unwrap().concat().len() == want.len()));
        let writes = writes.lock().unwrap();
        assert_eq!(writes.concat(), want, "the paste whole, then Enter, then what was typed");
        assert!(writes.iter().all(|w| w.len() <= INPUT_CHUNK));
        assert!(!failed.load(Ordering::Relaxed));
    }

    #[test]
    fn sent_text_is_a_paste_then_enter_and_typing_waits() {
        let clock = crate::testing::ManualClock::new(1);
        let (term, ctl) = FakeTerm::new(TermSize::new(80, 24), None);
        let host = TermHost::new(Box::new(term), TermSize::new(80, 24), clock);
        let text = long_text(64 * 1024);
        host.send_text(text.clone());
        host.write(b"typed").unwrap();
        let want = [paste_bytes(&text), b"\r".to_vec(), b"typed".to_vec()].concat();
        assert!(wait_until(|| ctl.input().len() == want.len()));
        assert_eq!(ctl.input(), want);
    }
}
