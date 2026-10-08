//! Headless terminal screen, one per agent, fed from the PTY reader thread.
//!
//! [`Screen`] is Pitwall's own interface; the emulator behind it is an
//! implementation detail so it can be swapped (it was `vt100`; it is
//! `alacritty_terminal` now; libghostty-vt may come later). Everything that
//! leaves this module is parser-neutral: plain text and the title for the
//! status rules, and a [`Snapshot`] (rows of styled runs) for drawing a
//! screen without a terminal emulator in the UI (the Wall).
//!
//! The emulator handles cursor movement, scroll regions, the alternate
//! screen, wide characters and resizing. Its parser keeps its state between
//! `feed` calls, so sequences split across PTY reads are reassembled. No
//! scrollback is kept (status rules read the visible screen only).
//!
//! Window titles (OSC 0 / OSC 2) are read by a small scanner of our own
//! ([`TitleScanner`]) so they keep exactly the bytes the program sent
//! (alacritty trims them).

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, NamedColor, Processor, Timeout};

/// Longest title we keep; anything longer is truncated (defensive bound).
const MAX_TITLE_CHARS: usize = 512;
/// Longest OSC payload the title scanner buffers.
const MAX_OSC_BYTES: usize = 4096;

pub struct Screen {
    term: Term<VoidListener>,
    parser: Processor<NoSync>,
    title: TitleScanner,
}

/// The emulator's size (no scrollback).
#[derive(Clone, Copy)]
struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
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

/// Synchronized updates (DEC mode 2026) are applied as they arrive instead
/// of being held back: a status rule should see what was drawn, and the
/// alacritty event loop that would expire a held update isn't used here.
#[derive(Default)]
struct NoSync;

impl Timeout for NoSync {
    fn set_timeout(&mut self, _: std::time::Duration) {}
    fn clear_timeout(&mut self) {}
    fn pending_timeout(&self) -> bool {
        false
    }
}

impl Screen {
    pub fn new(rows: u16, cols: u16) -> Self {
        let size = Size { rows: rows.max(1) as usize, cols: cols.max(1) as usize };
        let config = Config { scrolling_history: 0, ..Config::default() };
        Screen { term: Term::new(config, &size, VoidListener), parser: Processor::new(), title: TitleScanner::default() }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.title.scan(bytes);
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.term.resize(Size { rows: rows.max(1) as usize, cols: cols.max(1) as usize });
    }

    /// (rows, cols).
    pub fn size(&self) -> (u16, u16) {
        (self.term.screen_lines() as u16, self.term.columns() as u16)
    }

    /// Visible screen as plain text, one line per row, trailing spaces trimmed.
    pub fn text(&self) -> String {
        let grid = self.term.grid();
        let (rows, cols) = (grid.screen_lines(), grid.columns());
        let mut out = String::with_capacity(rows * (cols + 1));
        for r in 0..rows {
            if r > 0 {
                out.push('\n');
            }
            let start = out.len();
            let row = &grid[Line(r as i32)];
            for c in 0..cols {
                let cell = &row[Column(c)];
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                out.push(printable(cell.c));
                if let Some(zw) = cell.zerowidth() {
                    out.extend(zw);
                }
            }
            let trimmed = out[start..].trim_end_matches(' ').len();
            out.truncate(start + trimmed);
        }
        out
    }

    /// Last window title set via OSC 0/2, if any.
    pub fn title(&self) -> Option<String> {
        self.title.title.clone()
    }

    /// The visible screen as rows of styled runs (what a terminal would draw),
    /// plus the cursor when the program shows it.
    pub fn snapshot(&self) -> Snapshot {
        let grid = self.term.grid();
        let (rows, cols) = (grid.screen_lines(), grid.columns());
        let mut lines = Vec::with_capacity(rows);
        for r in 0..rows {
            let row = &grid[Line(r as i32)];
            // Like xterm: trailing blank default-background cells draw nothing.
            let mut end = cols;
            while end > 0 && is_blank(&row[Column(end - 1)]) {
                end -= 1;
            }
            let mut runs: Vec<Run> = Vec::new();
            let mut c = 0;
            while c < end {
                let cell = &row[Column(c)];
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    c += 1;
                    continue;
                }
                let wide = cell.flags.contains(Flags::WIDE_CHAR);
                let style = Style::of(cell.fg, cell.bg, cell.flags);
                let zw = cell.zerowidth().filter(|z| !z.is_empty());
                // A wide character (or one carrying combining marks) is a run
                // of its own, so the UI knows how many cells it spans.
                let alone = wide || zw.is_some();
                match runs.last_mut() {
                    Some(last) if !alone && !last.alone() && last.style == style => last.text.push(printable(cell.c)),
                    _ => {
                        let mut text = String::new();
                        text.push(printable(cell.c));
                        if let Some(zw) = zw {
                            text.extend(zw);
                        }
                        runs.push(Run { text, style, cells: if wide { 2 } else { 1 }, single: alone });
                    }
                }
                c += if wide { 2 } else { 1 };
            }
            lines.push(runs);
        }
        let cursor = (self.term.mode().contains(TermMode::SHOW_CURSOR) && grid.display_offset() == 0).then(|| {
            let p = grid.cursor.point;
            ((p.column.0).min(cols.saturating_sub(1)) as u16, p.line.0.max(0) as u16)
        });
        Snapshot { rows: rows as u16, cols: cols as u16, cursor, lines }
    }
}

/// What a cell shows as text: blanks and tabs (alacritty marks tab stops it
/// passed with '\t') are spaces.
fn printable(c: char) -> char {
    if c == '\t' || c == '\0' {
        ' '
    } else {
        c
    }
}

fn is_blank(cell: &alacritty_terminal::term::cell::Cell) -> bool {
    matches!(cell.c, ' ' | '\t' | '\0')
        && cell.zerowidth().is_none_or(|z| z.is_empty())
        && matches!(cell.bg, AColor::Named(NamedColor::Background))
        && !cell.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
}

// ── Snapshot: the screen for drawing ────────────────────────────────────────

/// A cell colour as the program set it; the UI resolves it with its theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// The theme's default foreground (for `fg`) or background (for `bg`).
    Default,
    /// Palette entry 0–255 (0–15: the theme's ANSI colours).
    Palette(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    fn of(c: AColor) -> Color {
        match c {
            AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
            AColor::Indexed(i) => Color::Palette(i),
            AColor::Named(n) => match n as usize {
                i @ 0..=15 => Color::Palette(i as u8),
                // Foreground/Background/Cursor and the Dim* variants (which
                // the parser itself never stores) are the defaults.
                _ => Color::Default,
            },
        }
    }
}

/// Text attributes, as bits (stable: they cross to the UI).
pub mod attr {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 1 << 1;
    pub const DIM: u16 = 1 << 2;
    pub const INVERSE: u16 = 1 << 3;
    pub const HIDDEN: u16 = 1 << 4;
    pub const STRIKE: u16 = 1 << 5;
    /// Underline style in bits 6–8: 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
    pub const UNDERLINE_SHIFT: u16 = 6;
    pub const UNDERLINE_MASK: u16 = 0b111 << UNDERLINE_SHIFT;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    /// [`attr`] bits.
    pub attrs: u16,
}

impl Style {
    fn of(fg: AColor, bg: AColor, f: Flags) -> Style {
        let mut a = 0;
        for (flag, bit) in [
            (Flags::BOLD, attr::BOLD),
            (Flags::ITALIC, attr::ITALIC),
            (Flags::DIM, attr::DIM),
            (Flags::INVERSE, attr::INVERSE),
            (Flags::HIDDEN, attr::HIDDEN),
            (Flags::STRIKEOUT, attr::STRIKE),
        ] {
            if f.contains(flag) {
                a |= bit;
            }
        }
        let underline = if f.contains(Flags::UNDERLINE) {
            1
        } else if f.contains(Flags::DOUBLE_UNDERLINE) {
            2
        } else if f.contains(Flags::UNDERCURL) {
            3
        } else if f.contains(Flags::DOTTED_UNDERLINE) {
            4
        } else if f.contains(Flags::DASHED_UNDERLINE) {
            5
        } else {
            0
        };
        a |= underline << attr::UNDERLINE_SHIFT;
        Style { fg: Color::of(fg), bg: Color::of(bg), attrs: a }
    }
}

/// Consecutive cells with one style.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// Cells this run's single character spans (2 for a wide character);
    /// 1 per character otherwise.
    pub cells: u8,
    single: bool,
}

impl Run {
    /// A run holding one character that must stay on its own (wide, or
    /// with combining marks).
    pub fn alone(&self) -> bool {
        self.single
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: u16,
    pub cols: u16,
    /// (col, row) of the cursor while the program shows it.
    pub cursor: Option<(u16, u16)>,
    /// One entry per row: its runs up to the last cell that draws anything.
    pub lines: Vec<Vec<Run>>,
}

// ── OSC 0 / OSC 2 titles ────────────────────────────────────────────────────

/// Follows the escape-sequence structure just enough to find OSC strings,
/// with the same rules as the VT parser (vte): an OSC ends at BEL or at any
/// ESC (ESC \ is the usual ST), CAN/SUB abort it, other C0 bytes inside are
/// ignored. Ordinary text is skipped with a byte search.
#[derive(Default)]
struct TitleScanner {
    state: Osc,
    buf: Vec<u8>,
    title: Option<String>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Osc {
    #[default]
    Ground,
    /// After ESC.
    Escape,
    /// Inside an OSC string.
    Body,
}

impl TitleScanner {
    fn scan(&mut self, bytes: &[u8]) {
        let mut i = 0;
        while i < bytes.len() {
            match self.state {
                Osc::Ground => match bytes[i..].iter().position(|&b| b == 0x1b) {
                    Some(p) => {
                        i += p + 1;
                        self.state = Osc::Escape;
                    }
                    None => return,
                },
                Osc::Escape => {
                    self.state = match bytes[i] {
                        b']' => {
                            self.buf.clear();
                            Osc::Body
                        }
                        0x1b => Osc::Escape,
                        _ => Osc::Ground,
                    };
                    i += 1;
                }
                Osc::Body => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        0x07 => {
                            self.dispatch();
                            self.state = Osc::Ground;
                        }
                        0x1b => {
                            self.dispatch();
                            self.state = Osc::Escape;
                        }
                        0x18 | 0x1a => self.state = Osc::Ground,
                        0x00..=0x1f => {}
                        _ => {
                            if self.buf.len() < MAX_OSC_BYTES {
                                self.buf.push(b);
                            }
                        }
                    }
                }
            }
        }
    }

    fn dispatch(&mut self) {
        let Some(sep) = self.buf.iter().position(|&b| b == b';') else {
            return;
        };
        let (kind, rest) = (&self.buf[..sep], &self.buf[sep + 1..]);
        if kind != b"0" && kind != b"2" {
            return;
        }
        let text = String::from_utf8_lossy(rest);
        self.title = Some(text.chars().filter(|c| !c.is_control()).take(MAX_TITLE_CHARS).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &Screen) -> Vec<String> {
        s.text().lines().map(str::to_string).collect()
    }

    #[test]
    fn plain_text_and_trailing_spaces() {
        let mut s = Screen::new(4, 20);
        s.feed(b"hello   \r\nworld");
        let l = lines(&s);
        assert_eq!(l[0], "hello");
        assert_eq!(l[1], "world");
        assert_eq!(s.text().split('\n').count(), 4);
    }

    #[test]
    fn cursor_moves_and_overwrite() {
        let mut s = Screen::new(5, 20);
        s.feed(b"\x1b[3;5Hmid\x1b[1;1Htop\x1b[3;5HMID");
        let l = lines(&s);
        assert_eq!(l[0], "top");
        assert_eq!(l[2], "    MID");
        // Erase line.
        s.feed(b"\x1b[1;1H\x1b[2K");
        assert_eq!(lines(&s)[0], "");
    }

    #[test]
    fn alternate_screen_round_trip() {
        let mut s = Screen::new(4, 20);
        s.feed(b"shell prompt $");
        s.feed(b"\x1b[?1049h\x1b[H\x1b[2Jfull screen app");
        assert!(s.text().contains("full screen app"));
        assert!(!s.text().contains("shell prompt"));
        s.feed(b"\x1b[?1049l");
        assert!(s.text().contains("shell prompt"));
        assert!(!s.text().contains("full screen app"));
    }

    #[test]
    fn wide_chars() {
        let mut s = Screen::new(2, 20);
        s.feed("日本語 ok ❯ ✻".as_bytes());
        assert_eq!(lines(&s)[0], "日本語 ok ❯ ✻");
    }

    #[test]
    fn utf8_split_across_feeds() {
        let mut s = Screen::new(2, 20);
        let bytes = "❯ hi".as_bytes();
        s.feed(&bytes[..1]);
        s.feed(&bytes[1..]);
        assert_eq!(lines(&s)[0], "❯ hi");
    }

    #[test]
    fn tabs_are_spaces() {
        let mut s = Screen::new(2, 30);
        s.feed(b"a\tb\t\tc");
        assert_eq!(lines(&s)[0], format!("a{}b{}c", " ".repeat(7), " ".repeat(15)));
    }

    #[test]
    fn resize_keeps_working() {
        let mut s = Screen::new(3, 10);
        s.feed(b"abcdefghij");
        s.resize(5, 40);
        s.feed(b"\x1b[5;1Hbottom");
        let l = lines(&s);
        assert_eq!(l.len(), 5);
        assert_eq!(l[4], "bottom");
        assert_eq!(s.size(), (5, 40));
        s.resize(0, 0); // clamped, must not panic
        s.feed(b"x");
    }

    #[test]
    fn osc_titles() {
        let mut s = Screen::new(2, 20);
        assert_eq!(s.title(), None);
        s.feed(b"\x1b]0;first\x07");
        assert_eq!(s.title().as_deref(), Some("first"));
        s.feed(b"\x1b]2;second\x1b\\");
        assert_eq!(s.title().as_deref(), Some("second"));
        // OSC 1 (icon name) does not change the title.
        s.feed(b"\x1b]1;icon\x07");
        assert_eq!(s.title().as_deref(), Some("second"));
        // Titles containing ';' survive.
        s.feed(b"\x1b]0;a;b;c\x07");
        assert_eq!(s.title().as_deref(), Some("a;b;c"));
        // Title text is not drawn on screen.
        assert_eq!(s.text().trim(), "");
    }

    #[test]
    fn osc_title_split_across_feeds() {
        let mut s = Screen::new(2, 30);
        let seq = "\x1b]0;✳ Claude Code\x07after".as_bytes();
        for chunk in seq.chunks(1) {
            s.feed(chunk);
        }
        assert_eq!(s.title().as_deref(), Some("✳ Claude Code"));
        assert_eq!(lines(&s)[0], "after");

        // Split in the middle of the ST terminator.
        s.feed(b"\x1b]2;spin");
        s.feed(b"ner \x1b");
        s.feed(b"\\");
        assert_eq!(s.title().as_deref(), Some("spinner "));
    }

    #[test]
    fn osc_title_aborted_or_interrupted() {
        let mut s = Screen::new(2, 30);
        s.feed(b"\x1b]0;one\x07");
        // CAN aborts the OSC: no new title.
        s.feed(b"\x1b]0;two\x18rest");
        assert_eq!(s.title().as_deref(), Some("one"));
        // Any ESC ends an OSC (vte dispatches it).
        s.feed(b"\x1b]0;three\x1b[1mbold");
        assert_eq!(s.title().as_deref(), Some("three"));
        // ESC ESC ] still starts an OSC.
        s.feed(b"\x1b\x1b]2;four\x07");
        assert_eq!(s.title().as_deref(), Some("four"));
    }

    #[test]
    fn garbage_does_not_panic() {
        let mut s = Screen::new(10, 30);
        let mut x: u32 = 0x1234_5678;
        let mut buf = Vec::new();
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let b = (x & 0xff) as u8;
            // Bias towards escape-heavy input.
            buf.push(if b.is_multiple_of(7) { 0x1b } else { b });
        }
        for chunk in buf.chunks(97) {
            s.feed(chunk);
        }
        s.resize(3, 5);
        s.feed(&buf[..5000]);
        let _ = s.text();
        let _ = s.title();
        let _ = s.snapshot();
    }

    // ── snapshots ──

    fn style(fg: Color, bg: Color, attrs: u16) -> Style {
        Style { fg, bg, attrs }
    }

    #[test]
    fn snapshot_runs_and_colours() {
        let mut s = Screen::new(3, 40);
        s.feed(b"plain \x1b[1;31mred\x1b[0m \x1b[38;5;208mo\x1b[38;2;1;2;3mrgb\x1b[0m\r\n");
        s.feed(b"\x1b[7minv\x1b[27m \x1b[4mu\x1b[4:3mc\x1b[0m \x1b[2;3mdi\x1b[0m\x1b[44m  \x1b[0m   ");
        let snap = s.snapshot();
        assert_eq!((snap.rows, snap.cols), (3, 40));
        let row0: Vec<(&str, Style)> = snap.lines[0].iter().map(|r| (r.text.as_str(), r.style)).collect();
        let d = Color::Default;
        assert_eq!(
            row0,
            vec![
                ("plain ", style(d, d, 0)),
                ("red", style(Color::Palette(1), d, attr::BOLD)),
                (" ", style(d, d, 0)),
                ("o", style(Color::Palette(208), d, 0)),
                ("rgb", style(Color::Rgb(1, 2, 3), d, 0)),
            ]
        );
        let row1: Vec<(&str, u16, Color)> = snap.lines[1].iter().map(|r| (r.text.as_str(), r.style.attrs, r.style.bg)).collect();
        assert_eq!(
            row1,
            vec![
                ("inv", attr::INVERSE, d),
                (" ", 0, d),
                ("u", 1 << attr::UNDERLINE_SHIFT, d),
                ("c", 3 << attr::UNDERLINE_SHIFT, d),
                (" ", 0, d),
                ("di", attr::DIM | attr::ITALIC, d),
                // Trailing default blanks are dropped, a coloured background is kept.
                ("  ", 0, Color::Palette(4)),
            ]
        );
        assert!(snap.lines[2].is_empty());
    }

    #[test]
    fn snapshot_wide_chars_and_cursor() {
        let mut s = Screen::new(2, 20);
        s.feed("a日本b".as_bytes());
        let snap = s.snapshot();
        let runs: Vec<(&str, u8)> = snap.lines[0].iter().map(|r| (r.text.as_str(), r.cells)).collect();
        assert_eq!(runs, vec![("a", 1), ("日", 2), ("本", 2), ("b", 1)]);
        assert_eq!(snap.cursor, Some((6, 0)));
        s.feed(b"\x1b[?25l");
        assert_eq!(s.snapshot().cursor, None);
        s.feed(b"\x1b[?25h\x1b[2;3H");
        assert_eq!(s.snapshot().cursor, Some((2, 1)));
    }

    /// What a busy Claude Code / Codex style TUI writes (cf. scripts/tui-agent.py):
    /// Ink-style erase + redraw of a live region, finished output above it,
    /// scroll regions and absolute moves, SGR of every kind, wide characters.
    fn tui_stream(frames: usize) -> Vec<Vec<u8>> {
        let spin = ['·', '✢', '✳', '✶', '✻', '✽'];
        let mut out = Vec::new();
        let mut prev = 0;
        for i in 0..frames {
            let mut f = String::new();
            if prev > 0 {
                f.push_str(&"\x1b[2K\x1b[1A".repeat(prev - 1));
                f.push_str("\x1b[2K\x1b[G");
            }
            if i % 8 == 0 {
                f.push_str(&format!("\x1b[38;2;95;211;141m⏺\x1b[0m \x1b[1mUpdate\x1b[0m(src/m{}.ts)\r\n", i % 50));
                for k in 0..(i % 5 + 2) {
                    f.push_str(&format!("      \x1b[48;2;34;92;43m{:>4} + const v{k} = f(\"日本語\");   \x1b[0m\r\n", 100 + k));
                }
                f.push_str("\tdone\t\x1b[4mlink\x1b[0m \x1b[7m inv \x1b[27m\r\n");
            }
            if i % 37 == 5 {
                // Codex-style: scroll region, absolute moves, insert/delete lines.
                f.push_str("\x1b7\x1b[2;8r\x1b[8;1H\r\nscrolled\x1b[r\x1b[3;4H\x1b[1L\x1b[2M\x1b[5X\x1b8");
            }
            let lines = [
                format!("\x1b[38;2;215;119;87m{} Working…\x1b[0m \x1b[2m({}s · esc to interrupt)\x1b[0m", spin[i % 6], i / 10),
                String::new(),
                format!("\x1b[38;5;245m╭{}╮\x1b[0m", "─".repeat(50)),
                format!("\x1b[38;5;245m│\x1b[0m \x1b[1m>\x1b[0m \x1b[3mtry \"fix it\"\x1b[0m{}\x1b[38;5;245m│\x1b[0m", " ".repeat(35)),
                format!("\x1b[38;5;245m╰{}╯\x1b[0m", "─".repeat(50)),
            ];
            prev = lines.len();
            f.push_str(&lines.join("\r\n"));
            out.push(f.into_bytes());
        }
        out
    }

    /// The status rules see the same text with this emulator as with vt100
    /// (the previous one) on realistic TUI output, frame by frame, at
    /// several sizes and with frames split at odd places.
    #[test]
    fn same_text_as_vt100() {
        for (rows, cols) in [(30u16, 100u16), (12, 60), (40, 200)] {
            let mut ours = Screen::new(rows, cols);
            let mut theirs = vt100::Parser::new(rows, cols, 0);
            for (i, frame) in tui_stream(300).iter().enumerate() {
                let cut = (i * 7) % frame.len().max(1);
                for part in [&frame[..cut], &frame[cut..]] {
                    ours.feed(part);
                    theirs.process(part);
                }
                let s = theirs.screen();
                let want: Vec<String> = s.rows(0, s.size().1).map(|r| r.trim_end().to_string()).collect();
                assert_eq!(ours.text(), want.join("\n"), "frame {i} at {rows}x{cols}");
            }
        }
    }
}
