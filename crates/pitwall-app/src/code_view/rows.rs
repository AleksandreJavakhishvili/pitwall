//! What the view draws, row by row: a file's lines, or a diff laid out
//! inline (deleted rows above inserted ones) or side by side (both sides on
//! one row, filler where a side has no line), with unchanged stretches
//! collapsed into "N hidden lines" rows (Monaco's hideUnchangedRegions: 3
//! lines of context, runs of 4 or more fold). Pure.

use std::collections::HashSet;
use std::ops::Range;

use super::diff::FileDiff;

/// Context kept next to a change.
pub const FOLD_MARGIN: usize = 3;
/// Fewer hidden lines than this are not worth a fold.
pub const FOLD_MIN: usize = 4;

/// Row heights (px): text lines, and the collapsed-region bar.
pub const LINE_H: f32 = 19.0;
pub const FOLD_H: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One file, no diff.
    File,
    /// Diff in one column.
    Unified,
    /// Diff side by side.
    Split,
}

/// Which version a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Split: the original's line on the left, the modified's on the right
    /// (`None`: filler). Unified: a context row has both, a deleted row only
    /// `old`, an inserted row only `new`. File: `new` only.
    Line {
        old: Option<usize>,
        new: Option<usize>,
        /// Part of a change.
        changed: bool,
    },
    /// Collapsed unchanged lines (`region` indexes [`Rows::regions`]).
    Fold { region: usize, hidden: usize },
}

impl Row {
    pub fn height(&self) -> f32 {
        match self {
            Row::Line { .. } => LINE_H,
            Row::Fold { .. } => FOLD_H,
        }
    }

    /// The line this row shows on `side`.
    pub fn line(&self, side: Side) -> Option<usize> {
        match (self, side) {
            (Row::Line { old, .. }, Side::Old) => *old,
            (Row::Line { new, .. }, Side::New) => *new,
            _ => None,
        }
    }
}

/// An unchanged stretch that can be collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// The rows of one view, with what is needed to find things in them.
#[derive(Debug, Clone, Default)]
pub struct Rows {
    pub rows: Vec<Row>,
    /// Top of each row (px), plus the total height at the end.
    tops: Vec<f32>,
    pub regions: Vec<Region>,
    /// First row of each change.
    pub hunk_rows: Vec<usize>,
    old_row: Vec<Option<usize>>,
    new_row: Vec<Option<usize>>,
}

impl Rows {
    /// Rows of a single file of `lines` lines.
    pub fn file(lines: usize) -> Rows {
        let rows = (0..lines)
            .map(|i| Row::Line {
                old: None,
                new: Some(i),
                changed: false,
            })
            .collect();
        Rows::finish(rows, Vec::new(), Vec::new(), 0, lines)
    }

    /// Rows of a diff; `expanded` regions are shown in full.
    pub fn diff(
        d: &FileDiff,
        old_lines: usize,
        new_lines: usize,
        layout: Layout,
        fold: bool,
        expanded: &HashSet<usize>,
    ) -> Rows {
        let regions = if fold {
            regions(d, old_lines, new_lines)
        } else {
            Vec::new()
        };
        let mut rows = Vec::new();
        let mut hunk_rows = Vec::new();
        let (mut o, mut n) = (0usize, 0usize);
        let mut next_region = 0;
        let mut equal = |rows: &mut Vec<Row>, o_end: usize, o: &mut usize, n: &mut usize| {
            while *o < o_end {
                if let Some(r) = regions.get(next_region).filter(|r| r.old.start == *o) {
                    let id = next_region;
                    next_region += 1;
                    if !expanded.contains(&id) {
                        rows.push(Row::Fold {
                            region: id,
                            hidden: r.old.len(),
                        });
                        *n += r.old.len();
                        *o = r.old.end;
                        continue;
                    }
                }
                rows.push(Row::Line {
                    old: Some(*o),
                    new: Some(*n),
                    changed: false,
                });
                *o += 1;
                *n += 1;
            }
        };
        for h in &d.hunks {
            equal(&mut rows, h.old.start, &mut o, &mut n);
            hunk_rows.push(rows.len());
            match layout {
                Layout::Split => {
                    let len = h.old.len().max(h.new.len());
                    for k in 0..len {
                        rows.push(Row::Line {
                            old: (k < h.old.len()).then(|| h.old.start + k),
                            new: (k < h.new.len()).then(|| h.new.start + k),
                            changed: true,
                        });
                    }
                }
                Layout::Unified | Layout::File => {
                    // An empty side's one line is no deleted/inserted row.
                    if !d.empty_old {
                        rows.extend(h.old.clone().map(|i| Row::Line {
                            old: Some(i),
                            new: None,
                            changed: true,
                        }));
                    }
                    if !d.empty_new {
                        rows.extend(h.new.clone().map(|i| Row::Line {
                            old: None,
                            new: Some(i),
                            changed: true,
                        }));
                    }
                }
            }
            o = h.old.end;
            n = h.new.end;
        }
        equal(&mut rows, old_lines, &mut o, &mut n);
        Rows::finish(rows, regions, hunk_rows, old_lines, new_lines)
    }

    fn finish(
        rows: Vec<Row>,
        regions: Vec<Region>,
        hunk_rows: Vec<usize>,
        old_lines: usize,
        new_lines: usize,
    ) -> Rows {
        let mut tops = Vec::with_capacity(rows.len() + 1);
        let mut y = 0.0;
        let mut old_row = vec![None; old_lines];
        let mut new_row = vec![None; new_lines];
        for (i, r) in rows.iter().enumerate() {
            tops.push(y);
            y += r.height();
            if let Row::Line { old, new, .. } = r {
                if let Some(o) = old.and_then(|o| old_row.get_mut(o)) {
                    *o = Some(i);
                }
                if let Some(n) = new.and_then(|n| new_row.get_mut(n)) {
                    *n = Some(i);
                }
            }
        }
        tops.push(y);
        Rows {
            rows,
            tops,
            regions,
            hunk_rows,
            old_row,
            new_row,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn height(&self) -> f32 {
        *self.tops.last().unwrap_or(&0.0)
    }

    pub fn top(&self, row: usize) -> f32 {
        self.tops[row.min(self.tops.len() - 1)]
    }

    /// The row at height `y` (clamped to the rows there are).
    pub fn row_at(&self, y: f32) -> usize {
        if self.rows.is_empty() {
            return 0;
        }
        let i = self.tops.partition_point(|t| *t <= y);
        i.saturating_sub(1).min(self.rows.len() - 1)
    }

    /// The row showing `line` of `side` (`None` while it is folded away).
    pub fn row_of(&self, side: Side, line: usize) -> Option<usize> {
        match side {
            Side::Old => self.old_row.get(line).copied().flatten(),
            Side::New => self.new_row.get(line).copied().flatten(),
        }
    }

    /// The region hiding `line` of `side`, if folded.
    pub fn region_hiding(&self, side: Side, line: usize) -> Option<usize> {
        if self.row_of(side, line).is_some() {
            return None;
        }
        self.regions.iter().position(|r| match side {
            Side::Old => r.old.contains(&line),
            Side::New => r.new.contains(&line),
        })
    }

    /// The first change after row `from` (wrapping to the first).
    pub fn next_hunk(&self, from: usize) -> Option<usize> {
        self.hunk_rows
            .iter()
            .copied()
            .find(|r| *r > from)
            .or_else(|| self.hunk_rows.first().copied())
    }

    /// The last change before row `from` (wrapping to the last).
    pub fn prev_hunk(&self, from: usize) -> Option<usize> {
        self.hunk_rows
            .iter()
            .rev()
            .copied()
            .find(|r| *r < from)
            .or_else(|| self.hunk_rows.last().copied())
    }
}

/// Collapsible unchanged stretches, in order.
fn regions(d: &FileDiff, old_lines: usize, new_lines: usize) -> Vec<Region> {
    let mut out = Vec::new();
    let (mut o, mut n) = (0usize, 0usize);
    let mut stretches = Vec::new();
    for h in &d.hunks {
        stretches.push((o, h.old.start, n, o == 0 && n == 0, false));
        o = h.old.end;
        n = h.new.end;
    }
    stretches.push((o, old_lines, n, d.hunks.is_empty(), true));
    let _ = new_lines;
    for (o0, o1, n0, at_start, at_end) in stretches {
        let len = o1.saturating_sub(o0);
        let keep_before = if at_start { 0 } else { FOLD_MARGIN };
        let keep_after = if at_end { 0 } else { FOLD_MARGIN };
        if len < keep_before + keep_after + FOLD_MIN {
            continue;
        }
        let a = o0 + keep_before;
        let b = o1 - keep_after;
        out.push(Region {
            old: a..b,
            new: n0 + keep_before..n0 + keep_before + (b - a),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::diff::diff;
    use super::*;

    fn text(n: usize) -> String {
        (0..n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn a_file_has_one_row_per_line() {
        let r = Rows::file(3);
        assert_eq!(r.len(), 3);
        assert_eq!(r.height(), 3.0 * LINE_H);
        assert_eq!(r.row_at(LINE_H * 1.5), 1);
        assert_eq!(r.row_at(1e9), 2);
        assert_eq!(r.row_of(Side::New, 2), Some(2));
    }

    #[test]
    fn unchanged_stretches_fold_with_three_lines_of_context() {
        let old = text(30);
        let new = old.replace("line 15\n", "line fifteen\n");
        let d = diff(&old, &new);
        let lines = 31; // 30 + the empty last line
        let r = Rows::diff(&d, lines, lines, Layout::Unified, true, &HashSet::new());
        // Before: lines 0..12 fold (12 lines), context 12..15, change, context 16..19, fold 19..31.
        assert_eq!(r.regions.len(), 2);
        assert_eq!(r.regions[0].old, 0..12);
        assert_eq!(r.regions[1].old, 19..31);
        assert_eq!(
            r.rows[0],
            Row::Fold {
                region: 0,
                hidden: 12
            }
        );
        let changed = r
            .rows
            .iter()
            .filter(|x| matches!(x, Row::Line { changed: true, .. }))
            .count();
        assert_eq!(changed, 2, "one deleted and one inserted row inline");
        assert_eq!(r.hunk_rows, [4]);
        assert_eq!(r.region_hiding(Side::New, 5), Some(0));
        assert_eq!(r.row_of(Side::New, 5), None);

        // Expanding the first region shows its lines.
        let r = Rows::diff(&d, lines, lines, Layout::Unified, true, &HashSet::from([0]));
        assert_eq!(r.row_of(Side::New, 5), Some(5));
        assert!(matches!(r.rows.last(), Some(Row::Fold { region: 1, .. })));
    }

    #[test]
    fn side_by_side_pads_the_shorter_side() {
        let d = diff("a\nx\nb\n", "a\ny1\ny2\ny3\nb\n");
        let r = Rows::diff(&d, 4, 6, Layout::Split, false, &HashSet::new());
        let fillers = r
            .rows
            .iter()
            .filter(|x| {
                matches!(
                    x,
                    Row::Line {
                        old: None,
                        new: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(fillers, 2);
        assert_eq!(r.len(), 1 + 3 + 2);
        assert_eq!(
            r.row_of(Side::Old, 2),
            r.row_of(Side::New, 4),
            "aligned after the change"
        );
    }

    #[test]
    fn small_stretches_stay_open_and_hunks_wrap() {
        let d = diff("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n");
        let r = Rows::diff(&d, 6, 6, Layout::Split, true, &HashSet::new());
        assert!(r.regions.is_empty());
        assert_eq!(r.hunk_rows.len(), 2);
        assert_eq!(r.next_hunk(0), Some(r.hunk_rows[1]));
        assert_eq!(r.next_hunk(r.hunk_rows[1]), Some(r.hunk_rows[0]), "wraps");
        assert_eq!(r.prev_hunk(r.hunk_rows[0]), Some(r.hunk_rows[1]));
    }

    #[test]
    fn heights_count_fold_rows() {
        let old = text(40);
        let new = format!("{old}tail\n");
        let d = diff(&old, &new);
        let r = Rows::diff(&d, 41, 42, Layout::Unified, true, &HashSet::new());
        assert!(matches!(r.rows[0], Row::Fold { .. }));
        assert_eq!(r.top(1), FOLD_H);
        assert_eq!(r.row_at(FOLD_H - 1.0), 0);
        assert_eq!(r.row_at(FOLD_H + 1.0), 1);
    }
}
