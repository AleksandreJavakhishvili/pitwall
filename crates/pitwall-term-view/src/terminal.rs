//! The terminal model: an `alacritty_terminal::Term` plus its parser, fed
//! from a byte stream on whatever thread delivers the bytes, and shared by
//! any number of views (an interactive pane and a Wall tile can show the
//! same terminal; there is one parser per terminal, not per view).
//!
//! GPUI-free on purpose: views subscribe to a coalesced "something changed"
//! signal and lock the state while they prepare a frame.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use futures::channel::mpsc;

use crate::theme::TermTheme;

/// A terminal's size in cells, plus the cell size in logical pixels (for
/// programs that ask for the window size in pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: f32,
    pub cell_height: f32,
}

impl TermSize {
    pub fn new(cols: u16, rows: u16) -> Self {
        TermSize { cols: cols.max(1), rows: rows.max(1), cell_width: 8.0, cell_height: 16.0 }
    }
}

/// Where a terminal's bytes come from and where its input and size go.
///
/// Output is pushed: [`attach`](TermStream::attach) gets a [`Feed`] and calls
/// [`Feed::push`] from any thread whenever bytes arrive (a PTY reader thread,
/// a holder connection, a test). Input ([`write`](TermStream::write)) and
/// size changes ([`resize`](TermStream::resize)) go back the same way.
/// Implementations must not block for long in `write` / `resize`: they are
/// called on the UI thread (and `write` also on the feeding thread, for
/// answers to the program's queries).
pub trait TermStream: Send + Sync + 'static {
    /// Start delivering output into `feed`. Called once, when the terminal
    /// is created. Call [`Feed::close`] when the stream ends.
    fn attach(&self, feed: Feed);
    /// Bytes for the program (keys, paste, mouse and focus reports, answers).
    fn write(&self, bytes: &[u8]);
    /// The terminal was resized by its view.
    fn resize(&self, size: TermSize);
}

/// A stream with no program behind it: output only comes from [`Feed`]
/// pushes; input and resizes are dropped. For read-only views and tests.
pub struct NullStream;

impl TermStream for NullStream {
    fn attach(&self, _feed: Feed) {}
    fn write(&self, _bytes: &[u8]) {}
    fn resize(&self, _size: TermSize) {}
}

/// What a terminal tells its interactive view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// OSC 0 / 2; `None` = reset.
    Title(Option<String>),
    Bell,
    /// OSC 52 store: the program put text on the clipboard.
    ClipboardStore(String),
    /// The byte stream ended.
    Exited,
}

/// Requests that need the UI (clipboard access).
pub(crate) enum UiRequest {
    Event(TermEvent),
    /// OSC 52 load: answer with the clipboard text, formatted.
    ClipboardLoad(Arc<dyn Fn(&str) -> String + Sync + Send + 'static>),
}

#[derive(Clone)]
pub struct TerminalConfig {
    /// Lines of scrollback (the web UI keeps 5 000).
    pub scrollback: usize,
    pub theme: TermTheme,
    /// OSC 52: whether programs may copy to (and read) the clipboard.
    pub osc52: Osc52,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        TerminalConfig { scrollback: 5_000, theme: TermTheme::default(), osc52: Osc52::OnlyCopy }
    }
}

/// The grid size alacritty needs.
#[derive(Clone, Copy)]
struct Dims {
    rows: usize,
    cols: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// One view's wake-up channel; `pending` coalesces signals until it runs.
struct Subscriber {
    tx: mpsc::UnboundedSender<()>,
    pending: Arc<AtomicBool>,
}

/// The side of the terminal the parser's event listener needs.
pub(crate) struct IoSide {
    stream: Box<dyn TermStream>,
    subscribers: Mutex<Vec<Subscriber>>,
    requests: Mutex<Vec<UiRequest>>,
    size: Mutex<TermSize>,
    theme: Mutex<TermTheme>,
    closed: AtomicBool,
    /// Bumped on every change; views compare it to skip idle frames.
    epoch: AtomicU64,
}

impl IoSide {
    fn wake(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|s| {
            if s.pending.swap(true, Ordering::AcqRel) {
                return !s.tx.is_closed();
            }
            s.tx.unbounded_send(()).is_ok()
        });
    }

    fn request(&self, r: UiRequest) {
        self.requests.lock().unwrap().push(r);
    }
}

/// The parser's event sink.
#[derive(Clone)]
pub struct Listener(Arc<IoSide>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let io = &self.0;
        match event {
            Event::PtyWrite(s) => io.stream.write(s.as_bytes()),
            Event::ColorRequest(index, format) => {
                let theme = io.theme.lock().unwrap();
                let rgb = match index {
                    0..=255 => theme.palette()[index],
                    256 => theme.foreground,
                    257 => theme.background,
                    _ => theme.cursor,
                };
                drop(theme);
                io.stream.write(format(rgb).as_bytes());
            }
            Event::TextAreaSizeRequest(format) => {
                let s = *io.size.lock().unwrap();
                let ws = WindowSize {
                    num_lines: s.rows,
                    num_cols: s.cols,
                    cell_width: s.cell_width.round() as u16,
                    cell_height: s.cell_height.round() as u16,
                };
                io.stream.write(format(ws).as_bytes());
            }
            Event::Title(t) => io.request(UiRequest::Event(TermEvent::Title(Some(t)))),
            Event::ResetTitle => io.request(UiRequest::Event(TermEvent::Title(None))),
            Event::Bell => io.request(UiRequest::Event(TermEvent::Bell)),
            Event::ClipboardStore(_, text) => io.request(UiRequest::Event(TermEvent::ClipboardStore(text))),
            Event::ClipboardLoad(_, format) => io.request(UiRequest::ClipboardLoad(format)),
            Event::CursorBlinkingChange | Event::MouseCursorDirty | Event::Wakeup => {}
            Event::Exit | Event::ChildExit(_) => {}
        }
    }
}

/// Parser state, behind the terminal's lock.
pub struct TermState {
    pub(crate) term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
    /// Per viewport line: bumped whenever the line is damaged.
    gens: Vec<u64>,
    next_gen: u64,
}

impl TermState {
    pub fn term(&self) -> &Term<Listener> {
        &self.term
    }

    /// Fold the emulator's damage into per-line generations (shared by every
    /// view), then reset it. Returns the generation of each viewport line.
    pub(crate) fn line_gens(&mut self) -> &[u64] {
        let rows = self.term.screen_lines();
        if self.gens.len() != rows {
            self.gens = vec![0; rows];
            self.next_gen += 1;
            let g = self.next_gen;
            self.gens.iter_mut().for_each(|x| *x = g);
        }
        self.next_gen += 1;
        let g = self.next_gen;
        match self.term.damage() {
            TermDamage::Full => self.gens.iter_mut().for_each(|x| *x = g),
            TermDamage::Partial(lines) => {
                for d in lines {
                    if let Some(x) = self.gens.get_mut(d.line) {
                        *x = g;
                    }
                }
            }
        }
        self.term.reset_damage();
        &self.gens
    }

    /// Apply a synchronized update (DEC 2026) that timed out.
    fn expire_sync(&mut self) {
        if let Some(deadline) = self.parser.sync_timeout().sync_timeout() {
            if Instant::now() >= deadline {
                self.parser.stop_sync(&mut self.term);
            }
        }
    }
}

struct Shared {
    state: FairMutex<TermState>,
    io: Arc<IoSide>,
}

/// A terminal. Cheap to clone; all clones are the same terminal.
#[derive(Clone)]
pub struct Terminal {
    shared: Arc<Shared>,
}

/// Pushes output into a terminal. Holds the terminal weakly: pushes after
/// the terminal is dropped are ignored.
#[derive(Clone)]
pub struct Feed {
    shared: Weak<Shared>,
}

/// Bytes are parsed in slices this big, releasing the lock in between so a
/// flood of output can't stall a frame.
const PARSE_SLICE: usize = 32 * 1024;

impl Feed {
    pub fn push(&self, bytes: &[u8]) {
        let Some(shared) = self.shared.upgrade() else { return };
        for slice in bytes.chunks(PARSE_SLICE) {
            let mut st = shared.state.lock_unfair();
            let st = &mut *st;
            st.parser.advance(&mut st.term, slice);
        }
        shared.io.wake();
    }

    /// The stream ended (the program exited or the connection closed).
    pub fn close(&self) {
        let Some(shared) = self.shared.upgrade() else { return };
        if !shared.io.closed.swap(true, Ordering::AcqRel) {
            shared.io.request(UiRequest::Event(TermEvent::Exited));
            shared.io.wake();
        }
    }
}

/// A view's change signal: resolves (coalesced) after the terminal changed.
pub struct Changes {
    rx: mpsc::UnboundedReceiver<()>,
    pending: Arc<AtomicBool>,
}

impl Changes {
    /// Wait for the next change. `false` once the terminal is gone.
    pub async fn next(&mut self) -> bool {
        use futures::StreamExt;
        let got = self.rx.next().await.is_some();
        self.pending.store(false, Ordering::Release);
        got
    }
}

impl Terminal {
    /// A terminal on `stream`, `size` cells big.
    pub fn new(stream: impl TermStream, size: TermSize, config: TerminalConfig) -> Terminal {
        let io = Arc::new(IoSide {
            stream: Box::new(stream),
            subscribers: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            size: Mutex::new(size),
            theme: Mutex::new(config.theme.clone()),
            closed: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
        });
        let tconf = Config { scrolling_history: config.scrollback, osc52: config.osc52, ..Config::default() };
        let dims = Dims { rows: size.rows.max(1) as usize, cols: size.cols.max(1) as usize };
        let term = Term::new(tconf, &dims, Listener(io.clone()));
        let state = TermState { term, parser: Processor::new(), gens: Vec::new(), next_gen: 0 };
        let shared = Arc::new(Shared { state: FairMutex::new(state), io });
        let t = Terminal { shared };
        t.shared.io.stream.attach(t.feed());
        t
    }

    /// A handle that pushes output into this terminal.
    pub fn feed(&self) -> Feed {
        Feed { shared: Arc::downgrade(&self.shared) }
    }

    /// Lock the parser state (briefly: the feeding thread waits meanwhile).
    pub fn lock(&self) -> impl std::ops::DerefMut<Target = TermState> + '_ {
        let mut st = self.shared.state.lock();
        st.expire_sync();
        st
    }

    /// When a pending synchronized update must be applied even without its end.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.shared.state.lock().parser.sync_timeout().sync_timeout()
    }

    /// Subscribe to change signals.
    pub fn changes(&self) -> Changes {
        let (tx, rx) = mpsc::unbounded();
        let pending = Arc::new(AtomicBool::new(false));
        self.shared.io.subscribers.lock().unwrap().push(Subscriber { tx, pending: pending.clone() });
        Changes { rx, pending }
    }

    /// Increases on every change.
    pub fn epoch(&self) -> u64 {
        self.shared.io.epoch.load(Ordering::Relaxed)
    }

    pub fn is_closed(&self) -> bool {
        self.shared.io.closed.load(Ordering::Acquire)
    }

    /// Send bytes to the program.
    pub fn write(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.shared.io.stream.write(bytes);
        }
    }

    pub fn size(&self) -> TermSize {
        *self.shared.io.size.lock().unwrap()
    }

    /// Resize the grid (reflowing) and tell the stream. No-op when the cell
    /// count is unchanged (the pixel size is still recorded).
    pub fn resize(&self, size: TermSize) {
        let size = TermSize { cols: size.cols.max(1), rows: size.rows.max(1), ..size };
        let old = std::mem::replace(&mut *self.shared.io.size.lock().unwrap(), size);
        if (old.cols, old.rows) == (size.cols, size.rows) {
            return;
        }
        {
            let mut st = self.shared.state.lock();
            st.term.resize(Dims { rows: size.rows as usize, cols: size.cols as usize });
        }
        self.shared.io.stream.resize(size);
        self.shared.io.wake();
    }

    /// Mode flags the input side needs.
    pub fn mode(&self) -> TermMode {
        *self.shared.state.lock().term.mode()
    }

    pub fn set_theme(&self, theme: TermTheme) {
        *self.shared.io.theme.lock().unwrap() = theme;
        self.shared.io.wake();
    }

    pub fn theme(&self) -> TermTheme {
        self.shared.io.theme.lock().unwrap().clone()
    }

    /// Scroll the viewport through the scrollback.
    pub fn scroll(&self, scroll: Scroll) {
        self.shared.state.lock().term.scroll_display(scroll);
        self.shared.io.wake();
    }

    pub(crate) fn take_requests(&self) -> Vec<UiRequest> {
        std::mem::take(&mut *self.shared.io.requests.lock().unwrap())
    }

    /// Signal views (after a change made through [`Terminal::lock`]).
    pub fn touch(&self) {
        self.shared.io.wake();
    }

    /// The visible screen as text (trailing spaces trimmed), for tests and tools.
    pub fn screen_text(&self) -> String {
        let st = self.lock();
        let term = &st.term;
        let grid = term.grid();
        let mut out = String::new();
        for line in 0..term.screen_lines() as i32 {
            let row = &grid[alacritty_terminal::index::Line(line - grid.display_offset() as i32)];
            let mut s = String::new();
            for col in 0..term.columns() {
                let cell = &row[alacritty_terminal::index::Column(col)];
                if cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                s.push(cell.c);
            }
            out.push_str(s.trim_end());
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what the terminal sends back.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>, Arc<Mutex<Vec<TermSize>>>);

    impl TermStream for Sink {
        fn attach(&self, _feed: Feed) {}
        fn write(&self, bytes: &[u8]) {
            self.0.lock().unwrap().extend_from_slice(bytes);
        }
        fn resize(&self, size: TermSize) {
            self.1.lock().unwrap().push(size);
        }
    }

    #[test]
    fn feeds_text_and_answers_queries() {
        let sink = Sink::default();
        let t = Terminal::new(sink.clone(), TermSize::new(20, 4), TerminalConfig::default());
        t.feed().push(b"hello\r\nw\xc3\xb6rld");
        assert_eq!(t.screen_text(), "hello\nwörld\n\n\n");
        // Device attributes and cursor position reports go back to the program.
        t.feed().push(b"\x1b[6n");
        assert_eq!(String::from_utf8(sink.0.lock().unwrap().clone()).unwrap(), "\x1b[2;6R");
        // OSC 11 query: answered with the theme background.
        sink.0.lock().unwrap().clear();
        t.feed().push(b"\x1b]11;?\x07");
        let answer = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(answer.contains("rgb:0d0d/0f0f/1212"), "{answer:?}");
    }

    #[test]
    fn split_sequences_are_reassembled() {
        let t = Terminal::new(NullStream, TermSize::new(10, 2), TerminalConfig::default());
        t.feed().push(b"\x1b[3");
        t.feed().push(b"1mred\x1b[0m");
        assert_eq!(t.screen_text(), "red\n\n");
    }

    #[test]
    fn resize_goes_to_the_stream_once() {
        let sink = Sink::default();
        let t = Terminal::new(sink.clone(), TermSize::new(10, 2), TerminalConfig::default());
        t.resize(TermSize::new(12, 3));
        t.resize(TermSize::new(12, 3));
        assert_eq!(sink.1.lock().unwrap().len(), 1);
        assert_eq!(t.lock().term.columns(), 12);
    }

    #[test]
    fn damage_becomes_line_generations() {
        let t = Terminal::new(NullStream, TermSize::new(10, 3), TerminalConfig::default());
        t.feed().push(b"\x1b[3;1H");
        t.lock().line_gens();
        let first = t.lock().line_gens().to_vec();
        t.feed().push(b"x"); // only the last line (where the cursor is) changes
        let second = t.lock().line_gens().to_vec();
        assert_eq!(first[0], second[0]);
        assert_eq!(first[1], second[1]);
        assert_ne!(first[2], second[2]);
    }

    #[test]
    fn changes_are_coalesced() {
        let t = Terminal::new(NullStream, TermSize::new(10, 3), TerminalConfig::default());
        let mut ch = t.changes();
        for _ in 0..100 {
            t.feed().push(b"x");
        }
        futures::executor::block_on(async {
            assert!(ch.next().await);
        });
        // One signal for all 100 pushes.
        assert!(ch.rx.try_recv().is_err());
        t.feed().close();
        assert!(t.is_closed());
        assert!(matches!(t.take_requests().as_slice(), [UiRequest::Event(TermEvent::Exited)]));
    }
}
