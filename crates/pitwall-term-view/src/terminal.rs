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

use crate::frozen::{ClearScan, Frozen};
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
    /// Freeze the scrollback once no view has locked the terminal for this
    /// long (`frozen.rs`).
    pub freeze_after: std::time::Duration,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        TerminalConfig {
            scrollback: 5_000,
            theme: TermTheme::default(),
            osc52: Osc52::OnlyCopy,
            freeze_after: FREEZE_AFTER,
        }
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
    /// When to give the rows' spare capacity back (after a width change).
    compact_at: Option<Instant>,
    /// The main screen still has rows to compact (it was behind the
    /// alternate screen when the active one was compacted).
    compact_main: bool,
    /// When a view last locked the terminal (drew, scrolled, selected…).
    seen_at: Instant,
    /// The scrollback while no view looks (`frozen.rs`).
    frozen: Frozen,
    clears: ClearScan,
    scrollback: usize,
    freeze_after: std::time::Duration,
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

    /// Compact the rows if a width change has settled. A program on the
    /// alternate screen keeps the main screen (and its scrollback) out of
    /// reach: that one is compacted once the program leaves it.
    fn maybe_compact(&mut self) {
        if self.compact_at.is_some_and(|at| Instant::now() >= at) {
            self.compact_at = None;
            compact_rows(&mut self.term);
            self.compact_main = self.term.mode().contains(TermMode::ALT_SCREEN);
        } else if self.compact_main && !self.term.mode().contains(TermMode::ALT_SCREEN) {
            self.compact_main = false;
            compact_rows(&mut self.term);
        }
    }

    /// Freeze the scrollback if no view has looked for a while: once it
    /// has [`FREEZE_BATCH`] rows, or all of it when `now`.
    fn maybe_freeze(&mut self, now: bool) {
        if self.seen_at.elapsed() >= self.freeze_after {
            let min = if now { 0 } else { FREEZE_BATCH };
            self.frozen.freeze(&mut self.term, min, self.scrollback);
        }
    }

    /// A view is looking: the whole scrollback back in the grid.
    fn seen(&mut self) {
        self.seen_at = Instant::now();
        if self.frozen.len() > 0 {
            self.frozen.thaw(&mut self.term);
        }
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

/// How long the width must stay the same before the rows are compacted.
const COMPACT_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// By default, a terminal no view has locked for this long (folded into a
/// chip, in a space not shown) keeps its scrollback compact (`frozen.rs`).
pub const FREEZE_AFTER: std::time::Duration = std::time::Duration::from_secs(15);

/// While output comes, the scrollback is frozen in batches this big:
/// alacritty makes spare rows 1 000 at a time, and they are given back
/// with each batch.
const FREEZE_BATCH: usize = 1_000;

impl Feed {
    pub fn push(&self, bytes: &[u8]) {
        let Some(shared) = self.shared.upgrade() else { return };
        for slice in bytes.chunks(PARSE_SLICE) {
            let mut st = shared.state.lock_unfair();
            let st = &mut *st;
            let cleared = st.frozen.len() > 0 && st.clears.saw_clear(slice);
            let mut slice = slice;
            if let Some(at) = alt_screen_at(slice).filter(|_| !st.term.mode().contains(TermMode::ALT_SCREEN)) {
                // The main screen's scrollback goes out of reach (and is
                // resized with the screen, rows and all) while a program
                // runs on the alternate screen: frozen until it is back.
                st.parser.advance(&mut st.term, &slice[..at]);
                st.frozen.freeze(&mut st.term, 0, st.scrollback);
                slice = &slice[at..];
            }
            st.parser.advance(&mut st.term, slice);
            if cleared {
                // The program cleared the scrollback: the frozen part too.
                st.frozen.clear();
            }
        }
        // A terminal no view draws (folded into a chip) compacts here.
        let mut st = shared.state.lock_unfair();
        st.maybe_compact();
        st.maybe_freeze(false);
        drop(st);
        shared.io.wake();
    }

    /// This terminal as its agent's screen for the engine
    /// ([`TerminalScreen`]).
    pub fn screen_source(&self) -> TerminalScreen {
        TerminalScreen { feed: self.clone(), title: pitwall_detect::screen::TitleScanner::default() }
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

/// A terminal as its agent's screen for the engine
/// (`pitwall_core::TermHost::share_screen`): the engine pushes the agent's
/// output through it and reads the status rules' text and the Wall's
/// styled rows from this terminal's grid, so an agent shown in a pane has
/// one parser, not one in the engine and another here. Reading never
/// counts as a view looking (nothing thaws, `frozen.rs`). Holds the
/// terminal weakly: once it is dropped, the engine goes back to a screen
/// of its own.
pub struct TerminalScreen {
    feed: Feed,
    /// OSC 0/2 titles exactly as sent (what the status rules match).
    title: pitwall_detect::screen::TitleScanner,
}

impl TerminalScreen {
    fn read<R>(&self, f: impl FnOnce(&Term<Listener>) -> R) -> Option<R> {
        let shared = self.feed.shared.upgrade()?;
        let mut st = shared.state.lock();
        st.expire_sync();
        Some(f(&st.term))
    }
}

impl pitwall_detect::ScreenSource for TerminalScreen {
    fn feed(&mut self, bytes: &[u8]) {
        self.title.scan(bytes);
        self.feed.push(bytes);
    }

    /// The view sizes its terminal (and, through its stream, the agent's).
    fn resize(&mut self, _rows: u16, _cols: u16) {}

    fn text(&self) -> String {
        self.read(pitwall_detect::screen::text).unwrap_or_default()
    }

    fn title(&self) -> Option<String> {
        self.title.title()
    }

    fn snapshot(&self) -> pitwall_detect::screen::Snapshot {
        self.read(pitwall_detect::screen::snapshot).unwrap_or(pitwall_detect::screen::Snapshot {
            rows: 0,
            cols: 0,
            cursor: None,
            lines: Vec::new(),
        })
    }

    fn alive(&self) -> bool {
        self.feed.shared.strong_count() > 0
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
        let state = TermState {
            term,
            parser: Processor::new(),
            gens: Vec::new(),
            next_gen: 0,
            compact_at: None,
            compact_main: false,
            seen_at: Instant::now(),
            frozen: Frozen::default(),
            clears: ClearScan::default(),
            scrollback: config.scrollback,
            freeze_after: config.freeze_after,
        };
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
    /// For views: the whole scrollback is in the grid while locked.
    pub fn lock(&self) -> impl std::ops::DerefMut<Target = TermState> + '_ {
        let mut st = self.shared.state.lock();
        st.seen();
        st.expire_sync();
        st.maybe_compact();
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
            st.seen(); // frozen rows have the old width
            st.term.resize(Dims { rows: size.rows as usize, cols: size.cols as usize });
            // Rows keep the allocation they had: narrower, the cells split
            // off; wider, Vec growth that doubles. Compact once the size
            // settles (not at every step of a drag).
            if size.cols != old.cols {
                st.compact_at = Some(Instant::now() + COMPACT_SETTLE);
            }
        }
        self.shared.io.stream.resize(size);
        self.shared.io.wake();
    }

    /// Freeze the scrollback now if no view has locked the terminal for
    /// a while (output does it too; this is for a quiet one).
    pub fn freeze_if_unseen(&self) {
        self.shared.state.lock().maybe_freeze(true);
    }

    /// Rows of scrollback kept compact (frozen) now.
    pub fn frozen_rows(&self) -> usize {
        self.shared.state.lock().frozen.len()
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
        self.lock().term.scroll_display(scroll);
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

/// Where `bytes` switch to the alternate screen (DECSET 1049, 1047, 47).
fn alt_screen_at(bytes: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = bytes[from..].windows(3).position(|w| w == b"\x1b[?") {
        let at = from + i;
        let rest = &bytes[at + 3..];
        if [&b"1049h"[..], b"1047h", b"47h"].iter().any(|p| rest.starts_with(p)) {
            return Some(at);
        }
        from = at + 3;
    }
    None
}

/// The width changed: alacritty keeps each row's allocation when it splits
/// cells off (narrower) and grows rows with `Vec` growth, which doubles
/// (wider), so a terminal resized a few times holds up to twice its width
/// in every row of its scrollback. Copy every row of the active screen
/// into an allocation of exactly its width (xterm.js also gives a line's
/// memory back when it shrinks).
fn compact_rows(term: &mut Term<Listener>) {
    use alacritty_terminal::grid::Row;
    use alacritty_terminal::index::Line;
    let grid = term.grid_mut();
    for line in grid.topmost_line().0..=grid.bottommost_line().0 {
        let row = &mut grid[Line(line)];
        let cells = row[..].to_vec();
        let n = cells.len();
        // Every cell counted as occupied: a reuse of the row clears it all.
        *row = Row::from_vec(cells, n);
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
    fn narrowing_keeps_the_text_and_the_scrollback() {
        let t = Terminal::new(NullStream, TermSize::new(40, 3), TerminalConfig::default());
        for i in 0..20 {
            t.feed().push(format!("line {i:02} {}\r\n", "x".repeat(20)).as_bytes());
        }
        t.resize(TermSize::new(20, 3));
        // Compacted once the width has settled.
        t.shared.state.lock().compact_at = Some(Instant::now());
        let st = t.lock();
        let term = st.term();
        assert_eq!(term.columns(), 20);
        // Reflowed (each line wraps once), still all there.
        assert!(term.history_size() >= 20, "{}", term.history_size());
        let top = &term.grid()[term.grid().topmost_line()];
        assert_eq!(top.len(), 20);
        drop(st);
        // Rows that were compacted are still cleared and reused correctly.
        t.feed().push(b"\x1b[2J\x1b[Hfresh");
        assert_eq!(t.screen_text(), "fresh\n\n\n");
    }

    fn lines(t: &Terminal, from: usize, n: usize) {
        for i in from..from + n {
            t.feed().push(format!("line {i}\r\n").as_bytes());
        }
    }

    /// A terminal that freezes as soon as no view looks.
    fn freezing(cols: u16) -> Terminal {
        let config = TerminalConfig { scrollback: 100, freeze_after: std::time::Duration::ZERO, ..TerminalConfig::default() };
        Terminal::new(NullStream, TermSize::new(cols, 3), config)
    }

    #[test]
    fn unseen_scrollback_freezes_and_comes_back_for_a_view() {
        let t = freezing(20);
        let other = Terminal::new(NullStream, TermSize::new(20, 3), TerminalConfig { scrollback: 100, ..TerminalConfig::default() });
        lines(&t, 0, 50);
        lines(&other, 0, 50);
        t.freeze_if_unseen();
        assert_eq!(t.frozen_rows(), 48);
        assert_eq!(t.shared.state.lock().term.history_size(), 0);
        // A view locks it: all back, as if nothing happened.
        let st = t.lock();
        assert_eq!(st.term().history_size(), 48);
        let top = |s: &TermState| s.term().grid()[s.term().grid().topmost_line()][..].iter().map(|c| c.c).collect::<String>();
        assert_eq!(top(&st), top(&other.lock()));
        assert_eq!(top(&st).trim_end(), "line 0");
        drop(st);
        assert_eq!(t.frozen_rows(), 0);
        assert_eq!(t.screen_text(), other.screen_text());
        // Kept to the scrollback limit.
        lines(&t, 50, 200);
        t.freeze_if_unseen();
        assert_eq!(t.frozen_rows(), 100);
        assert_eq!(t.lock().term().history_size(), 100);
    }

    #[test]
    fn clearing_the_scrollback_clears_the_frozen_part() {
        let t = freezing(20);
        lines(&t, 0, 30);
        t.freeze_if_unseen();
        assert!(t.frozen_rows() > 0);
        t.feed().push(b"\x1b[H\x1b[2J\x1b[");
        t.feed().push(b"3Jafter\r\n");
        lines(&t, 100, 2);
        // Only what came after the clear: "after" scrolled off since.
        assert_eq!(t.lock().term().history_size(), 1);
        assert_eq!(t.screen_text(), "line 100\nline 101\n\n");
    }

    #[test]
    fn the_main_screen_scrollback_freezes_under_a_full_screen_program() {
        // Seen all along: only the switch to the alternate screen freezes.
        let t = Terminal::new(NullStream, TermSize::new(20, 3), TerminalConfig { scrollback: 100, ..TerminalConfig::default() });
        lines(&t, 0, 30);
        t.feed().push(b"\x1b[?1049h\x1b[Hfull screen");
        assert_eq!(t.frozen_rows(), 28);
        assert_eq!(t.lock().term().history_size(), 0);
        // Still frozen while the program runs, even with a view looking.
        t.resize(TermSize::new(30, 3));
        assert_eq!(t.frozen_rows(), 28);
        t.feed().push(b"\x1b[?1049l");
        assert_eq!(t.lock().term().history_size(), 28);
        assert_eq!(t.frozen_rows(), 0);
        assert_eq!(alt_screen_at(b"ab\x1b[?25l\x1b[?47h"), Some(8));
        assert_eq!(alt_screen_at(b"\x1b[?1049l\x1b[?2004h"), None);
    }

    #[test]
    fn a_resize_thaws_first() {
        let t = freezing(20);
        lines(&t, 0, 30);
        t.freeze_if_unseen();
        assert!(t.frozen_rows() > 0);
        t.resize(TermSize::new(10, 3));
        assert_eq!(t.frozen_rows(), 0);
        // "line NN" fits in 10 columns: nothing reflowed away.
        assert_eq!(t.lock().term().history_size(), 28);
    }

    #[test]
    fn as_the_engines_screen_it_parses_once_and_reads_without_thawing() {
        use pitwall_detect::{Screen, ScreenSource};
        let t = freezing(20);
        let mut source = t.feed().screen_source();
        let mut engines = Screen::new(3, 20);
        let out = b"\x1b]0;made-up title\x07\x1b[1mbold\x1b[0m plain\r\nline 1\r\nline 2\r\nline 3";
        source.feed(out);
        engines.feed(out);
        t.freeze_if_unseen();
        assert_eq!(source.text(), engines.text());
        assert_eq!(source.title(), engines.title());
        assert_eq!(source.snapshot(), engines.snapshot());
        assert_eq!(t.frozen_rows(), 1, "reading is not a view looking");
        assert!(source.alive());
        drop(t);
        assert!(!source.alive());
        assert_eq!(source.text(), "");
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
