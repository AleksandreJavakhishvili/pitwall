//! Two versions of a file compared the way Review's Monaco/CodeMirror diff
//! did: lines first, inserted or deleted blocks slid to the best boundary
//! (by indentation, Monaco's `shiftSequenceDiffs`), then characters inside
//! each replaced block, tidied so changes don't fray into one-letter bits.
//! Pure; the lines are the source lines (`\n`-separated, `\r` kept so CRLF
//! changes show).

use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use similar::{capture_diff_slices_deadline, Algorithm, DiffOp};

/// Time the line diff may take before it settles for a coarser answer.
const LINE_DEADLINE: Duration = Duration::from_millis(1500);
/// The same for the characters of one block.
const CHAR_DEADLINE: Duration = Duration::from_millis(200);
/// Blocks larger than this (characters, both sides) get no character diff.
const MAX_CHAR_DIFF: usize = 40_000;

/// Lines `old` of the original replaced by lines `new` of the modified
/// (0-based, half-open; one side may be empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// What changed in one line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineMarks {
    /// The whole line was inserted or deleted (drawn in the stronger tint).
    pub full: bool,
    /// Changed bytes of the source line (empty when `full`).
    pub ranges: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileDiff {
    pub hunks: Vec<Hunk>,
    /// Changed lines of the original, by line index.
    pub old_marks: HashMap<usize, LineMarks>,
    /// Changed lines of the modified, by line index.
    pub new_marks: HashMap<usize, LineMarks>,
    pub added: usize,
    pub removed: usize,
    /// The original is empty (a new file) or the modified is (a deleted
    /// one). As in the React app (Monaco's rule), that side's one empty
    /// line is part of the change, not a removed or added line: it is
    /// tinted, never listed as a deleted or inserted row inline, and not
    /// counted.
    pub empty_old: bool,
    pub empty_new: bool,
}

impl FileDiff {
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

/// Source lines of a text (as `Document` splits it).
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

pub fn diff(old: &str, new: &str) -> FileDiff {
    let a = lines(old);
    let b = lines(new);
    let mut hunks = line_hunks(&a, &b);
    shift(&mut hunks, &a, &b);
    let mut out = FileDiff::default();
    for h in &hunks {
        out.removed += h.old.len();
        out.added += h.new.len();
        if h.old.is_empty() || h.new.is_empty() {
            for i in h.old.clone() {
                out.old_marks.insert(i, full());
            }
            for i in h.new.clone() {
                out.new_marks.insert(i, full());
            }
            continue;
        }
        let (om, nm) = char_marks(&a[h.old.clone()], &b[h.new.clone()]);
        out.old_marks.extend(h.old.clone().zip(om));
        out.new_marks.extend(h.new.clone().zip(nm));
    }
    out.hunks = hunks;
    out.empty_old = old.is_empty() && !new.is_empty();
    out.empty_new = new.is_empty() && !old.is_empty();
    if out.empty_old {
        out.removed = 0;
        out.old_marks.clear();
    }
    if out.empty_new {
        out.added = 0;
        out.new_marks.clear();
    }
    out
}

fn full() -> LineMarks {
    LineMarks {
        full: true,
        ranges: Vec::new(),
    }
}

fn line_hunks(a: &[&str], b: &[&str]) -> Vec<Hunk> {
    let ops =
        capture_diff_slices_deadline(Algorithm::Myers, a, b, Some(Instant::now() + LINE_DEADLINE));
    let mut out: Vec<Hunk> = Vec::new();
    for op in ops {
        let (o, n) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index..old_index + old_len, new_index..new_index),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index..old_index, new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index..old_index + old_len,
                new_index..new_index + new_len,
            ),
        };
        match out.last_mut() {
            Some(last) if last.old.end == o.start && last.new.end == n.start => {
                last.old.end = o.end;
                last.new.end = n.end;
            }
            _ => out.push(Hunk { old: o, new: n }),
        }
    }
    out
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn boundary(lines: &[&str], at: usize) -> i64 {
    let before = if at == 0 {
        0
    } else {
        indentation(lines[at - 1])
    };
    let after = if at >= lines.len() {
        0
    } else {
        indentation(lines[at])
    };
    1000 - (before + after) as i64
}

/// Slide pure insertions/deletions along equal lines to the best boundary
/// (Monaco: the earliest of the best-scoring places).
fn shift(hunks: &mut [Hunk], a: &[&str], b: &[&str]) {
    for i in 0..hunks.len() {
        let r = hunks[i].clone();
        let ins = r.old.is_empty();
        if ins == r.new.is_empty() {
            continue;
        }
        let (seq, s, e) = if ins {
            (b, r.new.start, r.new.end)
        } else {
            (a, r.old.start, r.old.end)
        };
        let prev_end = if i > 0 {
            if ins {
                hunks[i - 1].new.end
            } else {
                hunks[i - 1].old.end
            }
        } else {
            0
        };
        let next_start = if i + 1 < hunks.len() {
            if ins {
                hunks[i + 1].new.start
            } else {
                hunks[i + 1].old.start
            }
        } else {
            seq.len()
        };
        let mut before = 0;
        while s > before && s - before > prev_end && seq[s - before - 1] == seq[e - before - 1] {
            before += 1;
        }
        let mut after = 0;
        while e + after < next_start && seq[s + after] == seq[e + after] {
            after += 1;
        }
        if before == 0 && after == 0 {
            continue;
        }
        let other_seq = if ins { a } else { b };
        let other_start = if ins { r.old.start } else { r.new.start } as i64;
        let (mut best, mut best_score) = (0i64, i64::MIN);
        for d in -(before as i64)..=(after as i64) {
            let at = |x: usize| (x as i64 + d) as usize;
            let score = boundary(seq, at(s))
                + boundary(seq, at(e))
                + 2 * boundary(other_seq, (other_start + d) as usize);
            if score > best_score {
                (best, best_score) = (d, score);
            }
        }
        let mv =
            |x: &Range<usize>| (x.start as i64 + best) as usize..(x.end as i64 + best) as usize;
        hunks[i] = Hunk {
            old: mv(&r.old),
            new: mv(&r.new),
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Seg {
    Eq(usize),
    Ch { del: usize, ins: usize },
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Changed characters of a replaced block, per line on each side.
fn char_marks(a_lines: &[&str], b_lines: &[&str]) -> (Vec<LineMarks>, Vec<LineMarks>) {
    let ta = a_lines.join("\n");
    let tb = b_lines.join("\n");
    let ac: Vec<char> = ta.chars().collect();
    let bc: Vec<char> = tb.chars().collect();
    let (ca, cb) = if ac.len() + bc.len() > MAX_CHAR_DIFF {
        (vec![true; ac.len()], vec![true; bc.len()])
    } else {
        changed_chars(&ac, &bc)
    };
    (per_line(&ta, &ca), per_line(&tb, &cb))
}

/// Which characters of each side changed.
fn changed_chars(ac: &[char], bc: &[char]) -> (Vec<bool>, Vec<bool>) {
    let ops = capture_diff_slices_deadline(
        Algorithm::Myers,
        ac,
        bc,
        Some(Instant::now() + CHAR_DEADLINE),
    );
    let mut segs: Vec<Seg> = Vec::new();
    let push = |s: Seg, segs: &mut Vec<Seg>| match (segs.last_mut(), s) {
        (Some(Seg::Eq(n)), Seg::Eq(m)) => *n += m,
        (Some(Seg::Ch { del, ins }), Seg::Ch { del: d, ins: i }) => {
            *del += d;
            *ins += i;
        }
        _ => segs.push(s),
    };
    for op in ops {
        let s = match op {
            DiffOp::Equal { len, .. } => Seg::Eq(len),
            DiffOp::Delete { old_len, .. } => Seg::Ch {
                del: old_len,
                ins: 0,
            },
            DiffOp::Insert { new_len, .. } => Seg::Ch {
                del: 0,
                ins: new_len,
            },
            DiffOp::Replace {
                old_len, new_len, ..
            } => Seg::Ch {
                del: old_len,
                ins: new_len,
            },
        };
        push(s, &mut segs);
    }
    let segs = tidy(segs, ac, bc);
    let mut ca = vec![false; ac.len()];
    let mut cb = vec![false; bc.len()];
    let (mut pa, mut pb) = (0, 0);
    for s in segs {
        match s {
            Seg::Eq(n) => {
                pa += n;
                pb += n;
            }
            Seg::Ch { del, ins } => {
                ca[pa..pa + del].iter_mut().for_each(|c| *c = true);
                cb[pb..pb + ins].iter_mut().for_each(|c| *c = true);
                pa += del;
                pb += ins;
            }
        }
    }
    (ca, cb)
}

/// Make a character diff readable: a change that starts or ends inside a
/// word takes the whole word, and changes a character or two apart join.
fn tidy(mut segs: Vec<Seg>, ac: &[char], bc: &[char]) -> Vec<Seg> {
    // Start positions of each segment on both sides.
    let starts = |segs: &[Seg]| {
        let mut out = Vec::with_capacity(segs.len());
        let (mut pa, mut pb) = (0usize, 0usize);
        for s in segs {
            out.push((pa, pb));
            match *s {
                Seg::Eq(n) => {
                    pa += n;
                    pb += n;
                }
                Seg::Ch { del, ins } => {
                    pa += del;
                    pb += ins;
                }
            }
        }
        out
    };
    // Word expansion, against the equal runs around each change.
    let pos = starts(&segs);
    for i in 0..segs.len() {
        let Seg::Ch { del, ins } = segs[i] else {
            continue;
        };
        let (pa, pb) = pos[i];
        let first_word = (del > 0 && is_word(ac[pa])) || (ins > 0 && is_word(bc[pb]));
        let last_word =
            (del > 0 && is_word(ac[pa + del - 1])) || (ins > 0 && is_word(bc[pb + ins - 1]));
        // Backward into the equal run before.
        if i > 0 && first_word {
            if let Seg::Eq(n) = segs[i - 1] {
                let mut k = 0;
                while k < n && is_word(ac[pa - k - 1]) {
                    k += 1;
                }
                if k > 0 {
                    segs[i - 1] = Seg::Eq(n - k);
                    segs[i] = Seg::Ch {
                        del: del + k,
                        ins: ins + k,
                    };
                }
            }
        }
        // Forward into the equal run after.
        if i + 1 < segs.len() && last_word {
            if let (Seg::Eq(n), Seg::Ch { del, ins }) = (segs[i + 1], segs[i]) {
                // The run after starts where it did before any expansion.
                let after = pos[i + 1].0;
                let mut k = 0;
                while k < n && is_word(ac[after + k]) {
                    k += 1;
                }
                if k > 0 {
                    segs[i + 1] = Seg::Eq(n - k);
                    segs[i] = Seg::Ch {
                        del: del + k,
                        ins: ins + k,
                    };
                }
            }
        }
    }
    // Join changes across empty or tiny equal runs (not across line breaks).
    let pos = starts(&segs);
    let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
    let mut i = 0;
    while i < segs.len() {
        let s = segs[i];
        if let Seg::Eq(n) = s {
            let between = matches!(out.last(), Some(Seg::Ch { .. }))
                && matches!(segs.get(i + 1), Some(Seg::Ch { .. }));
            let pa = pos[i].0;
            let small = n == 0 || (n <= 2 && !ac[pa..pa + n].contains(&'\n'));
            if between && small {
                if let Some(Seg::Ch { del, ins }) = out.last_mut() {
                    *del += n;
                    *ins += n;
                }
                i += 1;
                continue;
            }
            if n == 0 {
                i += 1;
                continue;
            }
        }
        match (out.last_mut(), s) {
            (Some(Seg::Ch { del, ins }), Seg::Ch { del: d, ins: k }) => {
                *del += d;
                *ins += k;
            }
            _ => out.push(s),
        }
        i += 1;
    }
    out
}

/// Per-line marks from per-character flags of a block's text.
fn per_line(text: &str, changed: &[bool]) -> Vec<LineMarks> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut ci = 0; // character index into `changed`
    for (li, line) in lines.iter().enumerate() {
        let n = line.chars().count();
        let flags = &changed[ci..ci + n];
        let nl_before = li > 0 && changed[ci - 1];
        let nl_after = li + 1 < lines.len() && changed[ci + n];
        let all = flags.iter().all(|c| *c);
        if all && (nl_before || nl_after) {
            out.push(full());
        } else {
            let mut ranges: Vec<Range<usize>> = Vec::new();
            for ((b, ch), &c) in line.char_indices().zip(flags) {
                if !c {
                    continue;
                }
                let e = b + ch.len_utf8();
                match ranges.last_mut() {
                    Some(r) if r.end == b => r.end = e,
                    _ => ranges.push(b..e),
                }
            }
            out.push(LineMarks {
                full: false,
                ranges,
            });
        }
        ci += n + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(o: Range<usize>, n: Range<usize>) -> Hunk {
        Hunk { old: o, new: n }
    }

    #[test]
    fn identical_texts_have_no_hunks() {
        let d = diff("a\nb\n", "a\nb\n");
        assert!(d.is_empty());
        assert_eq!((d.added, d.removed), (0, 0));
    }

    #[test]
    fn a_changed_line_marks_only_the_changed_word() {
        let d = diff("let x = 1;\nkeep\n", "let y = 1;\nkeep\n");
        assert_eq!(d.hunks, [h(0..1, 0..1)]);
        let o = &d.old_marks[&0];
        let n = &d.new_marks[&0];
        assert!(!o.full && !n.full);
        assert_eq!(o.ranges, vec![4..5]);
        assert_eq!(n.ranges, vec![4..5]);
    }

    #[test]
    fn changes_inside_a_word_take_the_whole_word() {
        let d = diff("call(fooBar)\n", "call(fooBaz)\n");
        assert_eq!(d.old_marks[&0].ranges, vec![5..11]);
        assert_eq!(d.new_marks[&0].ranges, vec![5..11]);
    }

    #[test]
    fn inserted_and_deleted_lines_are_whole() {
        let d = diff("a\nb\n", "a\nnew\nb\n");
        assert_eq!(d.hunks, [h(1..1, 1..2)]);
        assert!(d.new_marks[&1].full);
        assert_eq!((d.added, d.removed), (1, 0));
        let d = diff("a\ngone\nb\n", "a\nb\n");
        assert_eq!(d.hunks, [h(1..2, 1..1)]);
        assert!(d.old_marks[&1].full);
    }

    #[test]
    fn new_and_deleted_files() {
        let d = diff("", "one\ntwo\n");
        assert_eq!(d.hunks, [h(0..0, 0..2)]);
        assert_eq!(d.added, 2);
        let d = diff("one\n", "");
        assert_eq!(d.hunks, [h(0..1, 0..0)]);
    }

    #[test]
    fn insertions_slide_to_the_indentation_boundary() {
        // A copied block lands on whole units, not "  a / } / f {".
        let old = "f {\n  a\n}\ng\n";
        let new = "f {\n  a\n}\nf {\n  a\n}\ng\n";
        let d = diff(old, new);
        assert_eq!(d.hunks.len(), 1);
        let hunk = &d.hunks[0];
        let added: Vec<_> = lines(new)[hunk.new.clone()].to_vec();
        assert_eq!(added, ["f {", "  a", "}"], "{hunk:?}");
    }

    #[test]
    fn a_new_file_has_no_removed_line() {
        // Without a final newline the empty original's one line is
        // "replaced"; it is still not a removed line.
        let d = diff("", "fn a() {}\nfn b() {}");
        assert_eq!((d.added, d.removed), (2, 0));
        assert!(d.empty_old && d.old_marks.is_empty());
        let d = diff("x\ny\n", "");
        assert_eq!((d.added, d.removed), (0, 2));
        assert!(d.empty_new);
    }

    #[test]
    fn a_replaced_block_marks_whole_extra_lines() {
        let d = diff("x = 1\n", "x = 2\nextra line\n");
        assert_eq!(d.hunks, [h(0..1, 0..2)]);
        assert!(!d.new_marks[&0].full);
        assert!(d.new_marks[&1].full, "{:?}", d.new_marks);
    }

    #[test]
    fn georgian_marks_are_on_character_boundaries() {
        let d = diff("სახელი: ანა\n", "სახელი: ნინო\n");
        let m = &d.new_marks[&0];
        let line = "სახელი: ნინო";
        for r in &m.ranges {
            assert!(line.is_char_boundary(r.start) && line.is_char_boundary(r.end));
        }
        assert_eq!(&line[m.ranges[0].clone()], "ნინო");
    }

    #[test]
    fn crlf_changes_show() {
        let d = diff("a\r\nb\r\n", "a\nb\n");
        assert_eq!(d.hunks.len(), 1);
    }
}
