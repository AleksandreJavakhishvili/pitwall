//! The scrollback of a terminal no view has looked at for a while, kept
//! compact ("frozen") until a view needs it again.
//!
//! alacritty keeps every row of the scrollback as a full row of 24-byte
//! cells, blanks included: 5 000 lines of a 140-column agent are ~17 MB,
//! twice what xterm.js kept. A terminal folded into a chip (or in a space
//! not shown) draws nothing, so its scrollback is moved out of the grid
//! into rows of text plus style runs (a typical line is ~100 bytes), and
//! moved back, in order and unchanged, the moment a view locks the
//! terminal again. Output keeps going into the (now short) grid meanwhile.
//!
//! Only the main screen's scrollback is frozen; a program on the alternate
//! screen keeps the main screen out of reach, so nothing moves while it
//! runs. A thawed row gets the terminal's width at that time (padded or
//! cut, no reflow): rows are thawed before every resize, so only a resize
//! during an alternate-screen program can make the widths differ.

use std::collections::VecDeque;
use std::sync::Arc;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Row};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, CellExtra, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Color;

/// A run of cells with the same attributes.
#[derive(Clone, Debug)]
struct Style {
    fg: Color,
    bg: Color,
    flags: Flags,
    extra: Option<Arc<CellExtra>>,
}

impl Style {
    fn of(cell: &Cell) -> Style {
        Style {
            fg: cell.fg,
            bg: cell.bg,
            flags: cell.flags,
            extra: cell.extra.clone(),
        }
    }

    fn matches(&self, cell: &Cell) -> bool {
        self.fg == cell.fg
            && self.bg == cell.bg
            && self.flags == cell.flags
            && match (&self.extra, &cell.extra) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

/// One row: its cells' characters, and their attributes as runs. Blank
/// cells at the end (as a fresh row has them) are left out.
#[derive(Debug)]
struct FrozenRow {
    text: Box<str>,
    runs: Box<[(u16, Style)]>,
}

fn is_blank(cell: &Cell) -> bool {
    cell.c == ' ' && cell.extra.is_none() && cell.flags.is_empty() && *cell == Cell::default()
}

impl FrozenRow {
    fn of(row: &Row<Cell>) -> FrozenRow {
        let cells = &row[..];
        let end = cells
            .iter()
            .rposition(|c| !is_blank(c))
            .map_or(0, |i| i + 1);
        let mut text = String::with_capacity(end);
        let mut runs: Vec<(u16, Style)> = Vec::new();
        for cell in &cells[..end] {
            text.push(cell.c);
            match runs.last_mut() {
                Some((n, style)) if style.matches(cell) && *n < u16::MAX => *n += 1,
                _ => runs.push((1, Style::of(cell))),
            }
        }
        FrozenRow {
            text: text.into_boxed_str(),
            runs: runs.into_boxed_slice(),
        }
    }

    /// Write the row into `row` (all of its cells).
    fn write_into(&self, row: &mut Row<Cell>) {
        let cols = row.len();
        let mut chars = self.text.chars();
        let mut col = 0;
        'runs: for (n, style) in self.runs.iter() {
            for _ in 0..*n {
                if col == cols {
                    break 'runs;
                }
                let Some(c) = chars.next() else { break 'runs };
                row[Column(col)] = Cell {
                    c,
                    fg: style.fg,
                    bg: style.bg,
                    flags: style.flags,
                    extra: style.extra.clone(),
                };
                col += 1;
            }
        }
        for i in col..cols {
            row[Column(i)] = Cell::default();
        }
    }
}

/// A terminal's frozen scrollback, oldest row first.
#[derive(Debug, Default)]
pub(crate) struct Frozen {
    rows: VecDeque<FrozenRow>,
}

impl Frozen {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// The program cleared the scrollback (or reset the terminal).
    pub fn clear(&mut self) {
        self.rows = VecDeque::new();
    }

    /// Move the grid's scrollback here, once it has at least `min` rows;
    /// keeps at most `cap` rows (the scrollback limit). Nothing moves while
    /// the view is scrolled up, text is selected or a program is on the
    /// alternate screen.
    pub fn freeze<L: EventListener>(&mut self, term: &mut Term<L>, min: usize, cap: usize) {
        if term.mode().contains(TermMode::ALT_SCREEN) || term.selection.is_some() {
            return;
        }
        let grid = term.grid_mut();
        let history = grid.history_size();
        if history == 0 || history < min || grid.display_offset() != 0 {
            return;
        }
        for i in (1..=history as i32).rev() {
            self.rows.push_back(FrozenRow::of(&grid[Line(-i)]));
        }
        while self.rows.len() > cap {
            self.rows.pop_front();
        }
        grid.clear_history();
        // And the rows alacritty keeps for reuse (it makes up to 1 000 at
        // a time as the scrollback grows, hence `min`).
        grid.truncate();
    }

    /// Put the frozen rows back into the grid's scrollback, before the rows
    /// that scrolled into it since; the screen is left as it is.
    pub fn thaw<L: EventListener>(&mut self, term: &mut Term<L>) {
        if self.rows.is_empty() || term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let grid = term.grid_mut();
        let (lines, cols) = (grid.screen_lines(), grid.columns());
        let history = grid.history_size();
        // Each row taken leaves a blank one: alacritty clears the rows it
        // scrolls in and can't clear an empty one.
        let take = |grid: &mut alacritty_terminal::Grid<Cell>, line: i32| {
            std::mem::replace(&mut grid[Line(line)], Row::new(cols))
        };
        let newer: Vec<Row<Cell>> = (1..=history as i32).rev().map(|i| take(grid, -i)).collect();
        let screen: Vec<Row<Cell>> = (0..lines as i32).map(|i| take(grid, i)).collect();
        grid.clear_history();
        // Each row goes to the top line, then scrolls into the scrollback.
        let region = Line(0)..Line(lines as i32);
        for row in std::mem::take(&mut self.rows) {
            if grid[Line(0)].len() != cols {
                grid[Line(0)] = Row::new(cols);
            }
            row.write_into(&mut grid[Line(0)]);
            grid.scroll_up(&region, 1);
        }
        for row in newer {
            grid[Line(0)] = row;
            grid.scroll_up(&region, 1);
        }
        for (i, row) in screen.into_iter().enumerate() {
            grid[Line(i as i32)] = row;
        }
    }
}

/// Watches output for what clears the scrollback (`CSI 3 J`, and `ESC c`,
/// a full reset), across chunk boundaries.
#[derive(Debug, Default)]
pub(crate) struct ClearScan {
    /// The last bytes seen (a sequence's start).
    tail: Vec<u8>,
}

impl ClearScan {
    const PATTERNS: [&'static [u8]; 2] = [b"\x1b[3J", b"\x1bc"];

    pub fn saw_clear(&mut self, bytes: &[u8]) -> bool {
        let mut seam = std::mem::take(&mut self.tail);
        seam.extend_from_slice(&bytes[..bytes.len().min(3)]);
        let found = Self::PATTERNS
            .iter()
            .any(|p| contains(&seam, p) || contains(bytes, p));
        // The tail of everything seen: of `bytes`, or of the seam when short.
        let all = if bytes.len() >= 3 { bytes } else { &seam[..] };
        self.tail = all[all.len().saturating_sub(3)..].to_vec();
        found
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    struct Size(usize, usize);
    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            self.0
        }
        fn screen_lines(&self) -> usize {
            self.0
        }
        fn columns(&self) -> usize {
            self.1
        }
    }

    /// A terminal and its parser.
    struct T(Term<VoidListener>, Processor);

    impl T {
        fn new() -> T {
            T::with_history(40)
        }
        fn with_history(scrolling_history: usize) -> T {
            let config = Config {
                scrolling_history,
                ..Config::default()
            };
            T(
                Term::new(config, &Size(4, 30), VoidListener),
                Processor::new(),
            )
        }
        fn feed(&mut self, bytes: &[u8]) {
            self.1.advance(&mut self.0, bytes);
        }
        /// Every row, scrollback first.
        fn rows(&self) -> Vec<Vec<Cell>> {
            let g = self.0.grid();
            (g.topmost_line().0..=g.bottommost_line().0)
                .map(|l| g[Line(l)][..].to_vec())
                .collect()
        }
    }

    /// The first cell that differs, if any.
    fn same(a: &[Vec<Cell>], b: &[Vec<Cell>]) {
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            for (j, (p, q)) in x.iter().zip(y).enumerate() {
                // Hyperlink ids are made per terminal: compare the URIs.
                let key = |c: &Cell| {
                    let link = c.hyperlink().map(|h| h.uri().to_string());
                    (
                        c.c,
                        c.fg,
                        c.bg,
                        c.flags,
                        c.zerowidth().map(<[char]>::to_vec),
                        c.underline_color(),
                        link,
                    )
                };
                assert_eq!(key(p), key(q), "row {i} column {j}");
            }
            assert_eq!(x.len(), y.len(), "row {i}");
        }
    }

    const OUTPUT: [&str; 4] = [
        "\x1b[1;31mred\x1b[0m plain \x1b[48;2;1;2;3mbg\x1b[0m\r\n\u{65e5}\u{672c} wide e\u{301}\r\n",
        "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ \x1b[4:3;58:5:2mcurly\x1b[0m\r\n\x1b[44m\x1b[K\x1b[0m\r\n",
        "wrapped: 0123456789012345678901234567890123456789\r\n",
        "prompt$ ",
    ];

    #[test]
    fn thawed_rows_are_the_rows_that_were_frozen() {
        // `a` freezes between outputs, `b` never does: same rows after a thaw.
        let (mut a, mut b) = (T::new(), T::new());
        let mut f = Frozen::default();
        for round in 0..3 {
            for (i, out) in OUTPUT.iter().enumerate() {
                for k in 0..=i * 3 {
                    let line = format!("round {round} line {k}\r\n");
                    a.feed(line.as_bytes());
                    b.feed(line.as_bytes());
                }
                a.feed(out.as_bytes());
                b.feed(out.as_bytes());
                f.freeze(&mut a.0, 0, 40);
                assert_eq!(a.0.grid().history_size(), 0);
            }
        }
        assert_eq!(f.len(), 40, "kept to the scrollback limit");
        // Output after the last freeze stays in the grid's own scrollback.
        for k in 0..6 {
            let line = format!("late {k}\r\n");
            a.feed(line.as_bytes());
            b.feed(line.as_bytes());
        }
        f.thaw(&mut a.0);
        assert_eq!(f.len(), 0);
        assert_eq!(a.0.grid().history_size(), b.0.grid().history_size());
        same(&a.rows(), &b.rows());
        assert_eq!(a.0.grid().cursor.point, b.0.grid().cursor.point);
        // And it keeps working like the other.
        a.feed(b"\r\nmore");
        b.feed(b"\r\nmore");
        same(&a.rows(), &b.rows());
    }

    #[test]
    fn thaws_after_much_output_while_frozen() {
        // A hidden agent that keeps printing: frozen, then more output than
        // the screen scrolls by, then seen again (the grid then holds
        // rows it made while frozen; a thaw used to leave empty ones that
        // panicked the next scroll).
        for (late, history, min) in [3, 5, 39, 40, 41, 120, 1500]
            .into_iter()
            .flat_map(|l| [(l, 40, 0), (l, 1000, 0), (l, 1000, 16)])
        {
            let (mut a, mut b) = (T::with_history(history), T::with_history(history));
            let mut f = Frozen::default();
            for k in 0..60 {
                let line = format!("early {k}\r\n");
                a.feed(line.as_bytes());
                b.feed(line.as_bytes());
                if k % 7 == 0 {
                    f.freeze(&mut a.0, min, history);
                }
            }
            f.freeze(&mut a.0, 0, history);
            for k in 0..late {
                let line = format!("late {k}\r\n");
                a.feed(line.as_bytes());
                b.feed(line.as_bytes());
            }
            f.thaw(&mut a.0);
            same(&a.rows(), &b.rows());
            for k in 0..50 {
                let line = format!("after {k}\r\n");
                a.feed(line.as_bytes());
                b.feed(line.as_bytes());
            }
            same(&a.rows(), &b.rows());
        }
    }

    #[test]
    fn nothing_moves_on_the_alternate_screen_or_scrolled_up() {
        let mut a = T::new();
        for k in 0..10 {
            a.feed(format!("line {k}\r\n").as_bytes());
        }
        let mut f = Frozen::default();
        a.0.scroll_display(alacritty_terminal::grid::Scroll::Delta(2));
        f.freeze(&mut a.0, 0, 40);
        assert_eq!(f.len(), 0);
        a.0.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        f.freeze(&mut a.0, 0, 40);
        assert_eq!(f.len(), 7);
        a.feed(b"\x1b[?1049hfull screen");
        f.thaw(&mut a.0);
        assert_eq!(f.len(), 7, "the main screen is out of reach");
        a.feed(b"\x1b[?1049l");
        f.thaw(&mut a.0);
        assert_eq!((f.len(), a.0.grid().history_size()), (0, 7));
    }

    #[test]
    fn clears_are_seen_across_chunks() {
        let mut s = ClearScan::default();
        assert!(!s.saw_clear(b"plain \x1b[2J text"));
        assert!(s.saw_clear(b"a\x1b[3Jb"));
        assert!(!s.saw_clear(b"x\x1b["));
        assert!(s.saw_clear(b"3J"));
        assert!(!s.saw_clear(b"\x1b"));
        assert!(!s.saw_clear(b"["));
        assert!(s.saw_clear(b"3J"));
        assert!(!s.saw_clear(b"\x1b"));
        assert!(s.saw_clear(b"c"));
    }
}
