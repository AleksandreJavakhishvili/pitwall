//! A Wall tile's copy of an agent's screen, kept from the backend's
//! [`ScreenFrame`]s (Tauri: `src/terminal/screenView.ts`), and the colour
//! rules that turn a run's `fg`/`bg`/`attrs` into what is drawn
//! (`src/terminal/screenStyle.ts`, which follows xterm's DOM renderer).
//!
//! No parser, no scrollback: a frame replaces the rows it carries. Rows are
//! kept pre-split into [`Segment`]s so painting is a straight walk: narrow
//! characters of one row share a segment drawn on the cell grid, wide and
//! clustered characters get their own segment at their column.

use std::sync::Arc;

use gpui::{rgb, Hsla, SharedString};

use pitwall_proto::{screen_attr as A, ScreenFrame, ScreenRun};

/// One run of cells sharing a style, inside a [`Segment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// Bytes of the segment's text.
    pub len: usize,
    /// Cells it covers.
    pub cells: u16,
    pub fg: u32,
    pub bg: u32,
    pub attrs: u16,
}

/// Characters drawn from one column on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub col: u16,
    pub cells: u16,
    pub text: SharedString,
    pub pieces: Vec<Piece>,
    /// One wide (two cells) or clustered character: not forced to the grid.
    pub single: bool,
}

/// One screen row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub segments: Vec<Segment>,
}

impl Row {
    pub fn from_runs(runs: &[ScreenRun]) -> Row {
        let mut segments: Vec<Segment> = Vec::new();
        let mut col: u16 = 0;
        for ScreenRun(text, fg, bg, attrs) in runs {
            if text.is_empty() {
                continue;
            }
            let hidden = attrs & A::HIDDEN != 0;
            if attrs & (A::WIDE | A::CLUSTER) != 0 {
                let cells = if attrs & A::WIDE != 0 { 2 } else { 1 };
                let text: SharedString = if hidden {
                    " ".repeat(cells as usize).into()
                } else {
                    text.clone().into()
                };
                segments.push(Segment {
                    col,
                    cells,
                    pieces: vec![Piece {
                        len: text.len(),
                        cells,
                        fg: *fg,
                        bg: *bg,
                        attrs: attrs & !(A::WIDE | A::CLUSTER),
                    }],
                    text,
                    single: true,
                });
                col = col.saturating_add(cells);
                continue;
            }
            let cells = text.chars().count() as u16;
            let shown = if hidden {
                " ".repeat(cells as usize)
            } else {
                text.clone()
            };
            let piece = Piece {
                len: shown.len(),
                cells,
                fg: *fg,
                bg: *bg,
                attrs: *attrs,
            };
            match segments.last_mut() {
                Some(s) if !s.single => {
                    s.text = format!("{}{shown}", s.text).into();
                    s.cells += cells;
                    s.pieces.push(piece);
                }
                _ => segments.push(Segment {
                    col,
                    cells,
                    text: shown.into(),
                    pieces: vec![piece],
                    single: false,
                }),
            }
            col = col.saturating_add(cells);
        }
        Row { segments }
    }

    /// The row's characters (tests and debugging).
    pub fn text(&self) -> String {
        let (mut out, mut at) = (String::new(), 0);
        for s in &self.segments {
            while at < s.col {
                out.push(' ');
                at += 1;
            }
            out.push_str(&s.text);
            at = s.col + s.cells;
        }
        out
    }
}

/// What a tile knows of its agent's screen.
#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    pub cursor: Option<(u16, u16)>,
    pub lines: Vec<Arc<Row>>,
    /// Frames applied (0: nothing to draw yet).
    pub frames: u64,
}

impl Screen {
    /// Apply one frame; the rows that changed (all of them on a full frame
    /// or a new size).
    pub fn apply(&mut self, f: &ScreenFrame) -> Vec<u16> {
        let resized = f.cols != self.cols || f.rows != self.rows;
        let mut dirty = Vec::new();
        if f.full || resized {
            let blank = Arc::new(Row::default());
            self.lines = (0..f.rows).map(|_| blank.clone()).collect();
            dirty.extend(0..f.rows);
        }
        self.cols = f.cols;
        self.rows = f.rows;
        for line in &f.lines {
            if line.0 < f.rows {
                self.lines[line.0 as usize] = Arc::new(Row::from_runs(&line.1));
                if !dirty.contains(&line.0) {
                    dirty.push(line.0);
                }
            }
        }
        if self.cursor != f.cursor {
            for (_, r) in [self.cursor, f.cursor].into_iter().flatten() {
                if r < f.rows && !dirty.contains(&r) {
                    dirty.push(r);
                }
            }
            self.cursor = f.cursor;
        }
        self.frames += 1;
        dirty
    }

    pub fn has_frame(&self) -> bool {
        self.frames > 0 && self.cols > 0 && self.rows > 0
    }
}

// ── colours ─────────────────────────────────────────────────────────────────

/// A terminal theme: default colours and the 256-colour palette.
#[derive(Debug, Clone, PartialEq)]
pub struct TermColors {
    pub fg: Hsla,
    pub bg: Hsla,
    pub cursor: Hsla,
    pub ansi: Vec<Hsla>,
}

/// The xterm themes of `src/terminal/registry.ts` (`DARK`, `LIGHT`): 16
/// colours, background, foreground, cursor.
const DARK: [u32; 19] = [
    0x1b1f25, 0xff6b6b, 0x5fd38d, 0xf2c35b, 0x6aa6ff, 0xc792ea, 0x5fc9d3, 0xd9dde3, //
    0x5b636f, 0xff8787, 0x7fe0a5, 0xffd479, 0x8cbcff, 0xd7aefb, 0x7fdde5, 0xf4f6f8, //
    0x0d0f12, 0xd9dde3, 0xe8ebef,
];
const LIGHT: [u32; 19] = [
    0x1d2128, 0xc92a2a, 0x2b8a3e, 0xa96800, 0x1c64d6, 0x8b3fc4, 0x0b7f8a, 0xd5d9de, //
    0x6b7280, 0xe03131, 0x37a24f, 0xc27c00, 0x3b7ee8, 0xa35ad8, 0x1898a4, 0xffffff, //
    0xfbfbfc, 0x1d2128, 0x1d2128,
];

/// xterm's 256 colours as `0xRRGGBB`: the theme's 16, the 6×6×6 cube, 24 greys.
pub fn palette(base16: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = base16.iter().take(16).copied().collect();
    let v = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    for i in 0..216 {
        out.push(v[(i / 36) % 6] << 16 | v[(i / 6) % 6] << 8 | v[i % 6]);
    }
    for i in 0..24u32 {
        let g = 8 + i * 10;
        out.push(g << 16 | g << 8 | g);
    }
    out
}

impl TermColors {
    fn from(t: &[u32; 19]) -> TermColors {
        TermColors {
            ansi: palette(&t[..16])
                .into_iter()
                .map(|c| rgb(c).into())
                .collect(),
            bg: rgb(t[16]).into(),
            fg: rgb(t[17]).into(),
            cursor: rgb(t[18]).into(),
        }
    }
    pub fn dark() -> TermColors {
        TermColors::from(&DARK)
    }
    pub fn light() -> TermColors {
        TermColors::from(&LIGHT)
    }
}

/// How one cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellStyle {
    pub fg: Hsla,
    pub bg: Option<Hsla>,
    pub bold: bool,
    pub italic: bool,
    /// 0 none, 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
    pub underline: u8,
    pub strike: bool,
}

/// xterm's `color.multiplyOpacity(c, 0.5)`.
fn half(c: Hsla) -> Hsla {
    Hsla { a: c.a * 0.5, ..c }
}

fn truecolor(v: u32) -> Hsla {
    rgb(v & 0xff_ffff).into()
}

/// A run's style as xterm's DOM renderer draws it (minimum contrast off):
/// inverse swaps, bold draws the first 8 colours bright, dim halves the
/// opacity of palette and default colours (not of truecolour).
pub fn resolve(fg_in: u32, bg_in: u32, attrs: u16, c: &TermColors) -> CellStyle {
    let inverse = attrs & A::INVERSE != 0;
    let (fg, bg) = if inverse {
        (bg_in, fg_in)
    } else {
        (fg_in, bg_in)
    };
    let dim = attrs & A::DIM != 0;
    let bold = attrs & A::BOLD != 0;
    let bg = if bg >= A::RGB {
        Some(truecolor(bg))
    } else if bg > 0 {
        c.ansi.get(bg as usize - 1).copied()
    } else if inverse {
        Some(c.fg)
    } else {
        None
    };
    let dimmed = |x: Hsla| if dim { half(x) } else { x };
    let fg = if fg >= A::RGB {
        truecolor(fg)
    } else if fg > 0 {
        let mut i = fg as usize - 1;
        if bold && i < 8 {
            i += 8;
        }
        dimmed(c.ansi.get(i).copied().unwrap_or(c.fg))
    } else if inverse {
        dimmed(c.bg)
    } else {
        dimmed(c.fg)
    };
    let strike = attrs & A::STRIKE != 0;
    let ul = ((attrs & A::UNDERLINE_MASK) >> A::UNDERLINE_SHIFT) as u8;
    CellStyle {
        fg,
        bg,
        bold,
        italic: attrs & A::ITALIC != 0,
        // xterm.css: strikethrough wins over underline.
        underline: if strike { 0 } else { ul },
        strike,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::ScreenLine;

    fn run(t: &str, fg: u32, bg: u32, attrs: u16) -> ScreenRun {
        ScreenRun(t.into(), fg, bg, attrs)
    }

    fn frame(full: bool, cols: u16, rows: u16, lines: Vec<(u16, Vec<ScreenRun>)>) -> ScreenFrame {
        ScreenFrame {
            cols,
            rows,
            cursor: None,
            full,
            lines: lines.into_iter().map(|(r, l)| ScreenLine(r, l)).collect(),
        }
    }

    #[test]
    fn the_palette_is_xterms() {
        let p = palette(&DARK[..16]);
        assert_eq!(p.len(), 256);
        assert_eq!(p[16], 0x000000);
        assert_eq!(p[21], 0x0000ff);
        assert_eq!(p[196], 0xff0000);
        assert_eq!(p[231], 0xffffff);
        assert_eq!(p[232], 0x080808);
        assert_eq!(p[255], 0xeeeeee);
    }

    #[test]
    fn the_themes_match_registry_ts() {
        let ts = include_str!("../../../../src/terminal/registry.ts");
        for hex in [
            "#0d0f12", "#d9dde3", "#5fd38d", "#fbfbfc", "#c92a2a", "#1898a4",
        ] {
            assert!(
                ts.contains(hex),
                "{hex} is gone from registry.ts: update wall/screen.rs"
            );
        }
        assert_eq!(TermColors::dark().bg, rgb(0x0d0f12).into());
        assert_eq!(TermColors::light().fg, rgb(0x1d2128).into());
    }

    #[test]
    fn colours_follow_xterm_rules() {
        let c = TermColors::dark();
        let plain = resolve(0, 0, 0, &c);
        assert_eq!((plain.fg, plain.bg), (c.fg, None));
        // Palette entry + 1; bold makes the first 8 bright.
        assert_eq!(resolve(2, 0, 0, &c).fg, c.ansi[1]);
        assert_eq!(resolve(2, 0, A::BOLD, &c).fg, c.ansi[9]);
        assert!(resolve(2, 0, A::BOLD, &c).bold);
        assert_eq!(
            resolve(200, 0, A::BOLD, &c).fg,
            c.ansi[199],
            "only the first 8"
        );
        // Truecolor, never dimmed.
        let tc = resolve(A::RGB | 0x102030, A::RGB | 0x405060, A::DIM, &c);
        assert_eq!(tc.fg, rgb(0x102030).into());
        assert_eq!(tc.bg, Some(rgb(0x405060).into()));
        // Dim halves palette and default colours.
        assert!((resolve(0, 0, A::DIM, &c).fg.a - 0.5).abs() < 0.01);
        // Inverse of defaults: theme fg behind, theme bg in front.
        let inv = resolve(0, 0, A::INVERSE, &c);
        assert_eq!((inv.fg, inv.bg), (c.bg, Some(c.fg)));
        let inv2 = resolve(3, 5, A::INVERSE, &c);
        assert_eq!((inv2.fg, inv2.bg), (c.ansi[4], Some(c.ansi[2])));
        // Strikethrough wins over underline.
        let s = resolve(0, 0, A::STRIKE | (1 << A::UNDERLINE_SHIFT), &c);
        assert!(s.strike && s.underline == 0);
        assert_eq!(resolve(0, 0, 3 << A::UNDERLINE_SHIFT, &c).underline, 3);
    }

    #[test]
    fn rows_split_into_grid_segments() {
        let row = Row::from_runs(&[
            run("ab", 0, 0, 0),
            run("cd", 2, 0, A::BOLD),
            run("日", 0, 0, A::WIDE),
            run("x", 0, 0, 0),
            run("pw", 0, 0, A::HIDDEN),
        ]);
        assert_eq!(row.segments.len(), 3);
        let s0 = &row.segments[0];
        assert_eq!((s0.col, s0.cells, s0.text.as_ref()), (0, 4, "abcd"));
        assert_eq!(s0.pieces.len(), 2);
        assert_eq!(s0.pieces[1].len, 2);
        let wide = &row.segments[1];
        assert!(wide.single);
        assert_eq!((wide.col, wide.cells), (4, 2));
        assert_eq!(wide.pieces[0].attrs, 0, "layout bits are not style");
        let s2 = &row.segments[2];
        assert_eq!(
            (s2.col, s2.text.as_ref()),
            (6, "x  "),
            "hidden text is blank"
        );
        assert_eq!(row.text(), "abcd日x  ");
    }

    #[test]
    fn frames_replace_the_rows_they_carry() {
        let mut s = Screen::default();
        assert!(!s.has_frame());
        let d = s.apply(&frame(
            true,
            10,
            3,
            vec![(0, vec![run("one", 0, 0, 0)]), (2, vec![run("$", 0, 0, 0)])],
        ));
        assert_eq!(d, vec![0, 1, 2]);
        assert!(s.has_frame());
        assert_eq!(s.lines[0].text(), "one");

        let d = s.apply(&frame(false, 10, 3, vec![(1, vec![run("two", 0, 0, 0)])]));
        assert_eq!(d, vec![1], "only the changed row");
        assert_eq!(s.lines[0].text(), "one");
        assert_eq!(s.lines[1].text(), "two");

        let mut f = frame(false, 10, 3, vec![]);
        f.cursor = Some((1, 2));
        assert_eq!(s.apply(&f), vec![2], "the cursor's row");

        // A new size starts over even without `full`.
        let d = s.apply(&frame(false, 20, 2, vec![(1, vec![run("z", 0, 0, 0)])]));
        assert_eq!(d, vec![0, 1]);
        assert_eq!(s.lines.len(), 2);
        assert_eq!(s.lines[0].text(), "");
        // Rows outside the screen are ignored.
        s.apply(&frame(false, 20, 2, vec![(5, vec![run("no", 0, 0, 0)])]));
        assert_eq!(s.lines.len(), 2);
    }
}
