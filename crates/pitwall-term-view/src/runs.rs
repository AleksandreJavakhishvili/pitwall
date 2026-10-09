//! One grid row → what to draw: background spans, text runs (cells sharing a
//! font style and colour, shaped as one string), decorations and
//! box-drawing cells. Pure: no GPUI, so it is unit-tested directly.
//!
//! Text runs carry, for every character, the column it starts at, so the
//! renderer can put each shaped glyph exactly on its cell (fallback fonts,
//! wide characters and combining marks never drift off the grid).

use std::hash::{Hash, Hasher};

use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, Rgb};

use crate::boxdraw;
use crate::theme::{Palette, Rgba8};

/// Font variant of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
}

/// Cells sharing a style, as one string to shape.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub style: FontStyle,
    pub color: Rgba8,
    /// `(byte offset in text, column, width in cells)` for each character
    /// that starts a cell (combining marks share their base's entry).
    pub cells: Vec<(u32, u16, u8)>,
}

impl TextRun {
    /// The column the character at byte `index` belongs to, and its width.
    pub fn cell_at(&self, index: usize) -> (u16, u8) {
        let i = self.cells.partition_point(|&(b, _, _)| b as usize <= index);
        let (_, col, w) = self.cells[i.saturating_sub(1)];
        (col, w)
    }
    pub fn start_col(&self) -> u16 {
        self.cells.first().map(|c| c.1).unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Underline,
    Double,
    Curly,
    Dotted,
    Dashed,
    Strike,
}

/// A decoration over columns `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoration {
    pub start: u16,
    pub end: u16,
    pub kind: LineKind,
    pub color: Rgba8,
}

/// A non-default background over columns `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BgSpan {
    pub start: u16,
    pub end: u16,
    pub color: Rgb,
}

/// A box-drawing / block character drawn as shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxCell {
    pub col: u16,
    pub ch: char,
    pub color: Rgba8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowRuns {
    pub backgrounds: Vec<BgSpan>,
    pub texts: Vec<TextRun>,
    pub decorations: Vec<Decoration>,
    pub boxes: Vec<BoxCell>,
}

/// Cheap identity of a row's drawable content (characters, colours, flags),
/// so a damaged row whose content did not change is not shaped again.
pub fn row_key(cells: &[Cell]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for cell in cells {
        cell.c.hash(&mut h);
        hash_color(cell.fg, &mut h);
        hash_color(cell.bg, &mut h);
        cell.flags.bits().hash(&mut h);
        if let Some(zw) = cell.zerowidth() {
            zw.hash(&mut h);
        }
        if let Some(uc) = cell.underline_color() {
            hash_color(uc, &mut h);
        }
    }
    h.finish()
}

fn hash_color(c: Color, h: &mut impl Hasher) {
    match c {
        Color::Named(n) => (0u8, n as u16).hash(h),
        Color::Spec(rgb) => (1u8, rgb.r, rgb.g, rgb.b).hash(h),
        Color::Indexed(i) => (2u8, i).hash(h),
    }
}

fn line_kind(flags: Flags) -> Option<LineKind> {
    if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Some(LineKind::Double)
    } else if flags.contains(Flags::UNDERCURL) {
        Some(LineKind::Curly)
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        Some(LineKind::Dotted)
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        Some(LineKind::Dashed)
    } else if flags.contains(Flags::UNDERLINE) {
        Some(LineKind::Underline)
    } else {
        None
    }
}

fn push_decoration(out: &mut Vec<Decoration>, d: Decoration) {
    if let Some(last) = out.last_mut() {
        if last.end == d.start && last.kind == d.kind && last.color == d.color {
            last.end = d.end;
            return;
        }
    }
    out.push(d);
}

/// Build the draw list for one row.
pub fn build_row(cells: &[Cell], pal: &Palette) -> RowRuns {
    let mut out = RowRuns::default();
    // Trailing blank cells with the default background draw nothing.
    let mut end = cells.len();
    while end > 0 && is_blank(&cells[end - 1]) {
        end -= 1;
    }
    let mut cur: Option<TextRun> = None;
    let mut col = 0usize;
    while col < end {
        let cell = &cells[col];
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            col += 1;
            continue;
        }
        let width: u8 = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        let (fg, bg) = pal.cell_colors(cell.fg, cell.bg, flags);
        let c16 = col as u16;
        let next = c16 + width as u16;

        if let Some(bg) = bg {
            match out.backgrounds.last_mut() {
                Some(last) if last.end == c16 && last.color == bg => last.end = next,
                _ => out.backgrounds.push(BgSpan { start: c16, end: next, color: bg }),
            }
        }

        let visible = fg.a != 0;
        if visible {
            if let Some(kind) = line_kind(flags) {
                let color =
                    cell.underline_color().map(|c| Rgba8::opaque(pal.resolve(c, false)).with_alpha(fg.a)).unwrap_or(fg);
                push_decoration(&mut out.decorations, Decoration { start: c16, end: next, kind, color });
            }
            if flags.contains(Flags::STRIKEOUT) {
                push_decoration(
                    &mut out.decorations,
                    Decoration { start: c16, end: next, kind: LineKind::Strike, color: fg },
                );
            }
        }

        let ch = cell.c;
        let blank = ch == ' ' || ch == '\t' || ch == '\0';
        if !visible || blank {
            // Invisible cells end nothing: a space inside a run stays in it
            // (keeps runs long); a run never starts with one.
            if let Some(run) = cur.as_mut() {
                if visible || blank {
                    run.cells.push((run.text.len() as u32, c16, width));
                    run.text.push(' ');
                }
            }
            col += width as usize;
            continue;
        }
        if boxdraw::is_drawn(ch) {
            if let Some(run) = cur.take() {
                out.texts.push(run);
            }
            out.boxes.push(BoxCell { col: c16, ch, color: fg });
            col += width as usize;
            continue;
        }
        let style = FontStyle { bold: flags.contains(Flags::BOLD), italic: flags.contains(Flags::ITALIC) };
        let same = matches!(&cur, Some(run) if run.style == style && run.color == fg);
        if !same {
            if let Some(run) = cur.take() {
                out.texts.push(run);
            }
            cur = Some(TextRun { text: String::new(), style, color: fg, cells: Vec::new() });
        }
        let run = cur.as_mut().expect("run");
        run.cells.push((run.text.len() as u32, c16, width));
        run.text.push(ch);
        if let Some(zw) = cell.zerowidth() {
            run.text.extend(zw.iter());
        }
        col += width as usize;
    }
    if let Some(run) = cur.take() {
        out.texts.push(run);
    }
    for run in &mut out.texts {
        // Spaces appended after the last visible character.
        let trimmed = run.text.trim_end_matches(' ').len();
        if trimmed < run.text.len() {
            run.text.truncate(trimmed);
            run.cells.retain(|&(b, _, _)| (b as usize) < trimmed);
        }
    }
    out
}

fn is_blank(cell: &Cell) -> bool {
    (cell.c == ' ' || cell.c == '\0')
        && matches!(cell.bg, Color::Named(alacritty_terminal::vte::ansi::NamedColor::Background))
        && !cell.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{hex, TermTheme};
    use alacritty_terminal::vte::ansi::NamedColor;

    fn pal() -> Palette {
        Palette::new(&TermTheme::pitwall_dark(), None)
    }

    fn cell(c: char) -> Cell {
        Cell { c, ..Cell::default() }
    }

    fn row(s: &str) -> Vec<Cell> {
        s.chars().map(cell).collect()
    }

    #[test]
    fn plain_text_is_one_run_and_trailing_blanks_are_dropped() {
        let r = build_row(&row("hello world   "), &pal());
        assert_eq!(r.texts.len(), 1);
        assert_eq!(r.texts[0].text, "hello world");
        assert_eq!(r.texts[0].cells.len(), 11);
        assert_eq!(r.texts[0].cell_at(6), (6, 1));
        assert!(r.backgrounds.is_empty());
    }

    #[test]
    fn style_changes_split_runs_and_spaces_dont() {
        let mut cells = row("ab cd");
        for c in &mut cells[3..] {
            c.fg = Color::Named(NamedColor::Red);
        }
        let r = build_row(&cells, &pal());
        assert_eq!(r.texts.len(), 2);
        assert_eq!(r.texts[0].text, "ab");
        assert_eq!(r.texts[1].text, "cd");
        assert_eq!(r.texts[1].start_col(), 3);
        assert_eq!(r.texts[1].color, Rgba8::opaque(hex(0xff6b6b)));

        let mut cells = row("a b");
        cells[1].fg = Color::Named(NamedColor::Blue); // an invisible difference
        assert_eq!(build_row(&cells, &pal()).texts.len(), 1);

        let mut cells = row("ab");
        cells[1].flags = Flags::BOLD;
        let r = build_row(&cells, &pal());
        assert_eq!(r.texts.len(), 2);
        assert!(r.texts[1].style.bold);
    }

    #[test]
    fn wide_characters_take_two_columns() {
        // "a中b": the wide char occupies columns 1 and 2.
        let mut cells = vec![cell('a'), cell('中'), cell(' '), cell('b')];
        cells[1].flags = Flags::WIDE_CHAR;
        cells[2].flags = Flags::WIDE_CHAR_SPACER;
        let r = build_row(&cells, &pal());
        assert_eq!(r.texts.len(), 1);
        let run = &r.texts[0];
        assert_eq!(run.text, "a中b");
        assert_eq!(run.cells, vec![(0, 0, 1), (1, 1, 2), (4, 3, 1)]);
        assert_eq!(run.cell_at(1), (1, 2));
        assert_eq!(run.cell_at(4), (3, 1));
    }

    #[test]
    fn georgian_and_combining_marks_stay_on_their_cells() {
        let mut cells = row("გამარჯობა e");
        // e + combining acute in one cell.
        cells[10].push_zerowidth('\u{301}');
        let r = build_row(&cells, &pal());
        let run = &r.texts[0];
        assert_eq!(run.text, "გამარჯობა e\u{301}");
        // Georgian letters are 3 bytes each, one column each.
        assert_eq!(run.cell_at(3), (1, 1));
        let e = run.text.find('e').unwrap();
        assert_eq!(run.cell_at(e), (10, 1));
        assert_eq!(run.cell_at(e + 1), (10, 1)); // the mark
    }

    #[test]
    fn backgrounds_merge_and_inverse_paints_one() {
        let mut cells = row("xy  z");
        for c in &mut cells[..4] {
            c.bg = Color::Indexed(4);
        }
        cells[4].flags = Flags::INVERSE;
        let r = build_row(&cells, &pal());
        assert_eq!(r.backgrounds.len(), 2);
        assert_eq!((r.backgrounds[0].start, r.backgrounds[0].end), (0, 4));
        assert_eq!(r.backgrounds[0].color, hex(0x6aa6ff));
        assert_eq!(r.backgrounds[1].color, hex(0xd9dde3));
        // Inverse text is drawn in the background colour.
        assert_eq!(r.texts.last().unwrap().color, Rgba8::opaque(hex(0x0d0f12)));
    }

    #[test]
    fn decorations_merge_across_spaces() {
        let mut cells = row("a b");
        for c in &mut cells {
            c.flags = Flags::UNDERLINE;
        }
        cells[2].flags = Flags::UNDERLINE | Flags::STRIKEOUT;
        let r = build_row(&cells, &pal());
        let ul: Vec<_> = r.decorations.iter().filter(|d| d.kind == LineKind::Underline).collect();
        assert_eq!(ul.len(), 1);
        assert_eq!((ul[0].start, ul[0].end), (0, 3));
        assert!(r.decorations.iter().any(|d| d.kind == LineKind::Strike && d.start == 2));
        let mut cells = row("ab");
        cells[0].flags = Flags::UNDERCURL;
        assert_eq!(build_row(&cells, &pal()).decorations[0].kind, LineKind::Curly);
    }

    #[test]
    fn hidden_and_box_cells() {
        let mut cells = row("a─b");
        cells[2].flags = Flags::HIDDEN;
        let r = build_row(&cells, &pal());
        assert_eq!(r.texts.len(), 1);
        assert_eq!(r.texts[0].text, "a");
        assert_eq!(r.boxes, vec![BoxCell { col: 1, ch: '─', color: Rgba8::opaque(hex(0xd9dde3)) }]);
    }

    #[test]
    fn key_changes_with_content_only() {
        let a = row("same");
        let mut b = row("same");
        assert_eq!(row_key(&a), row_key(&b));
        b[0].fg = Color::Indexed(3);
        assert_ne!(row_key(&a), row_key(&b));
    }
}
