//! The sequence layout (layout-lld.md §5): no graph layout, participants
//! left to right, one row per step, frames around the rows they hold.

use super::{Rect, SLACK, SMALL_PX};
use crate::mermaid::sequence::{BranchKind, FrameKind, NotePlacement, Sequence, Step};
use crate::text::{text_width, Weight};

/// A participant's header: its box at the top and its lifeline's x.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub rect: Rect,
    pub x: f64,
}

/// One drawn step.
#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    /// A message from `from_x` to `to_x` at `y`, its label above; a
    /// message to itself loops to the right down to `y2`.
    Message { step: usize, from_x: f64, to_x: f64, y: f64, y2: f64, label_x: f64, label_y: f64, label: String },
    Note { step: usize, rect: Rect, text: String },
}

/// A frame (`loop`, `alt`…): its rect, its tag and its branches'
/// dividers (y and label).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub kind: FrameKind,
    pub label: String,
    pub rect: Rect,
    pub branches: Vec<(f64, String)>,
}

/// A `box` around participants.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub label: String,
    pub rect: Rect,
}

/// A laid-out sequence diagram.
#[derive(Debug, Clone, PartialEq)]
pub struct SeqPlaced {
    pub headers: Vec<Header>,
    pub rows: Vec<Row>,
    pub frames: Vec<Frame>,
    pub activations: Vec<Rect>,
    pub groups: Vec<Group>,
    /// Where the lifelines end.
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

/// A header's height.
pub const HEADER_H: f64 = 36.0;
/// Room above the headers for a `box`'s label.
const GROUP_BAND: f64 = 24.0;
const HEADER_PAD: f64 = 14.0;
const HEADER_MIN_W: f64 = 96.0;
const GAP: f64 = 32.0;
const MESSAGE_ROW: f64 = 36.0;
const SELF_ROW: f64 = 52.0;
const NOTE_H: f64 = 28.0;
const FRAME_ROW: f64 = 28.0;
const FRAME_INSET: f64 = 8.0;

fn width(text: &str) -> f64 {
    text_width(text, SMALL_PX, Weight::Regular) * SLACK
}

/// The kind word a frame's tag shows.
pub fn frame_word(kind: FrameKind) -> &'static str {
    match kind {
        FrameKind::Loop => "loop",
        FrameKind::Alt => "alt",
        FrameKind::Opt => "opt",
        FrameKind::Par => "par",
        FrameKind::Critical => "critical",
        FrameKind::Break => "break",
        FrameKind::Rect => "",
    }
}

fn branch_word(kind: BranchKind) -> &'static str {
    match kind {
        BranchKind::Else => "else",
        BranchKind::And => "and",
        BranchKind::Option => "option",
    }
}

/// Lays out `s`.
pub fn layout(s: &Sequence) -> SeqPlaced {
    let n = s.participants.len();
    let top = if s.boxes.is_empty() { 0.0 } else { GROUP_BAND };
    let widths: Vec<f64> =
        s.participants.iter().map(|p| (text_width(&p.label, 13.0, Weight::SemiBold) * SLACK + 2.0 * HEADER_PAD).max(HEADER_MIN_W)).collect();
    // Gaps between neighbouring centres, from the headers, then widened
    // for the messages that span them.
    let mut gaps: Vec<f64> = (1..n).map(|i| widths[i - 1] / 2.0 + widths[i] / 2.0 + GAP).collect();
    let mut number = 0;
    let label_of = |text: &str, number: &mut u32| -> String {
        if s.autonumber {
            *number += 1;
            format!("{number}. {text}")
        } else {
            text.to_string()
        }
    };
    let mut labels: Vec<Option<String>> = Vec::new();
    for step in &s.steps {
        labels.push(match step {
            Step::Message { text, .. } => Some(label_of(text, &mut number)),
            _ => None,
        });
    }
    let mut spans: Vec<(usize, usize, f64)> = Vec::new();
    for (step, label) in s.steps.iter().zip(&labels) {
        match step {
            Step::Message { from, to, .. } if from == to => {
                let need = width(label.as_deref().unwrap_or("")) + 40.0;
                if *from + 1 < n {
                    spans.push((*from, *from + 1, need + widths[*from + 1] / 2.0));
                }
            }
            Step::Message { from, to, .. } => {
                let (a, b) = (*from.min(to), *from.max(to));
                spans.push((a, b, width(label.as_deref().unwrap_or("")) + 24.0));
            }
            Step::Note { placement: NotePlacement::Over, first, second: Some(second), text, .. } => {
                let (a, b) = (*first.min(second), *first.max(second));
                spans.push((a, b, width(text) + 16.0 - 40.0));
            }
            _ => {}
        }
    }
    spans.sort_by_key(|s| s.1 - s.0);
    for (a, b, need) in spans {
        if a == b || b > gaps.len() {
            continue;
        }
        let have: f64 = gaps[a..b].iter().sum();
        if have < need {
            gaps[b - 1] += need - have;
        }
    }
    let mut centres = Vec::with_capacity(n);
    let mut x = widths.first().map_or(0.0, |w| w / 2.0);
    for i in 0..n {
        if i > 0 {
            x += gaps[i - 1];
        }
        centres.push(x);
    }
    let headers: Vec<Header> = centres.iter().zip(&widths).map(|(&c, &w)| Header { rect: Rect { x: c - w / 2.0, y: top, w, h: HEADER_H }, x: c }).collect();

    let mut y = top + HEADER_H + 20.0;
    let mut rows = Vec::new();
    let mut frames = Vec::new();
    // Open frames: index into `frames`, and the participants touched.
    let mut open: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut active: Vec<Vec<f64>> = vec![Vec::new(); n];
    let mut activations = Vec::new();
    let touch = |open: &mut Vec<(usize, Vec<usize>)>, ps: &[usize]| {
        for (_, t) in open.iter_mut() {
            t.extend_from_slice(ps);
        }
    };
    let deactivate = |active: &mut Vec<Vec<f64>>, activations: &mut Vec<Rect>, p: usize, y: f64| {
        if let Some(start) = active.get_mut(p).and_then(|a| a.pop()) {
            let depth = active[p].len() as f64;
            activations.push(Rect { x: centres[p] - 5.0 + depth * 4.0, y: start, w: 10.0, h: (y - start).max(8.0) });
        }
    };
    for (i, step) in s.steps.iter().enumerate() {
        match step {
            Step::Message { from, to, activate_target, deactivate_source, .. } => {
                let label = labels[i].clone().unwrap_or_default();
                touch(&mut open, &[*from, *to]);
                let (fx, tx) = (centres[*from], centres[*to]);
                if from == to {
                    rows.push(Row::Message { step: i, from_x: fx, to_x: fx, y: y + 18.0, y2: y + 36.0, label_x: fx + 8.0, label_y: y + 8.0, label });
                    y += SELF_ROW;
                } else {
                    rows.push(Row::Message { step: i, from_x: fx, to_x: tx, y: y + 22.0, y2: y + 22.0, label_x: (fx + tx) / 2.0, label_y: y + 8.0, label });
                    y += MESSAGE_ROW;
                }
                if *activate_target {
                    active[*to].push(y - MESSAGE_ROW / 2.0);
                }
                if *deactivate_source {
                    deactivate(&mut active, &mut activations, *from, y - MESSAGE_ROW / 2.0);
                }
            }
            Step::Note { placement, first, second, text, .. } => {
                let w = width(text) + 16.0;
                let c = centres[*first];
                let rect = match (placement, second) {
                    (NotePlacement::LeftOf, _) => Rect { x: c - 12.0 - w, y, w, h: NOTE_H },
                    (NotePlacement::RightOf, _) => Rect { x: c + 12.0, y, w, h: NOTE_H },
                    (NotePlacement::Over, Some(second)) => {
                        let (a, b) = (c.min(centres[*second]), c.max(centres[*second]));
                        let w = w.max(b - a + 40.0);
                        Rect { x: (a + b) / 2.0 - w / 2.0, y, w, h: NOTE_H }
                    }
                    (NotePlacement::Over, None) => Rect { x: c - w / 2.0, y, w, h: NOTE_H },
                };
                touch(&mut open, &[*first]);
                if let Some(second) = second {
                    touch(&mut open, &[*second]);
                }
                rows.push(Row::Note { step: i, rect, text: text.clone() });
                y += NOTE_H + 10.0;
            }
            Step::Activate { participant, .. } => active[*participant].push(y),
            Step::Deactivate { participant, .. } => deactivate(&mut active, &mut activations, *participant, y),
            Step::FrameStart { kind, label, .. } => {
                frames.push(Frame { kind: *kind, label: label.clone(), rect: Rect { x: 0.0, y, w: 0.0, h: 0.0 }, branches: Vec::new() });
                open.push((frames.len() - 1, Vec::new()));
                y += FRAME_ROW;
            }
            Step::FrameBranch { kind, label, .. } => {
                if let Some((f, _)) = open.last() {
                    let words = [branch_word(*kind), label.as_str()].iter().filter(|w| !w.is_empty()).copied().collect::<Vec<_>>().join(" ");
                    frames[*f].branches.push((y, words));
                }
                y += FRAME_ROW - 4.0;
            }
            Step::FrameEnd { .. } => {
                y += 4.0;
                close(&mut frames, &mut open, &centres, &widths, y);
                y += 8.0;
            }
        }
    }
    while !open.is_empty() {
        close(&mut frames, &mut open, &centres, &widths, y);
        y += 8.0;
    }
    let bottom = y + 12.0;
    for p in 0..n {
        while !active[p].is_empty() {
            deactivate(&mut active, &mut activations, p, bottom);
        }
    }
    let mut groups = Vec::new();
    for (g, b) in s.boxes.iter().enumerate() {
        let members: Vec<&Header> = s.participants.iter().zip(&headers).filter(|(p, _)| p.box_group == Some(g)).map(|(_, h)| h).collect();
        let (Some(first), Some(last)) = (members.first(), members.last()) else { continue };
        let x0 = members.iter().map(|h| h.rect.x).fold(first.rect.x, f64::min) - 10.0;
        let x1 = members.iter().map(|h| h.rect.right()).fold(last.rect.right(), f64::max) + 10.0;
        groups.push(Group { label: b.label.clone(), rect: Rect { x: x0, y: 0.0, w: x1 - x0, h: bottom + 8.0 } });
    }
    let mut right = headers.last().map_or(0.0, |h| h.rect.right());
    let mut left: f64 = 0.0;
    for r in &rows {
        match r {
            Row::Message { from_x, label_x, label, .. } => {
                right = right.max(from_x + 40.0).max(label_x + width(label) / 2.0);
                left = left.min(label_x - width(label) / 2.0);
            }
            Row::Note { rect, .. } => {
                right = right.max(rect.right());
                left = left.min(rect.x);
            }
        }
    }
    for f in &frames {
        right = right.max(f.rect.right());
        left = left.min(f.rect.x);
    }
    for g in &groups {
        right = right.max(g.rect.right());
        left = left.min(g.rect.x);
    }
    // Everything from x 0: move it all right by what spills over the left.
    let mut placed = SeqPlaced { headers, rows, frames, activations, groups, bottom, width: right - left, height: bottom + 8.0 };
    if left < 0.0 {
        shift(&mut placed, -left);
    }
    placed
}

/// Closes the innermost open frame at `y`: across the participants its
/// steps touched (all of them when none), 8 px further in per level.
fn close(frames: &mut [Frame], open: &mut Vec<(usize, Vec<usize>)>, centres: &[f64], widths: &[f64], y: f64) {
    let Some((f, touched)) = open.pop() else { return };
    let depth = open.len() as f64;
    let (a, b) = if touched.is_empty() {
        (0, centres.len().saturating_sub(1))
    } else {
        (*touched.iter().min().unwrap_or(&0), *touched.iter().max().unwrap_or(&0))
    };
    let margin = (widths.get(a).copied().unwrap_or(HEADER_MIN_W) / 2.0).max(48.0) - depth * FRAME_INSET;
    let x0 = centres.get(a).copied().unwrap_or(0.0) - margin;
    let margin_b = (widths.get(b).copied().unwrap_or(HEADER_MIN_W) / 2.0).max(48.0) - depth * FRAME_INSET;
    let x1 = centres.get(b).copied().unwrap_or(0.0) + margin_b;
    let r = &mut frames[f].rect;
    (r.x, r.w, r.h) = (x0, x1 - x0, y - r.y);
    // The parents hold what the child touched.
    if let Some((_, parent)) = open.last_mut() {
        parent.extend(touched);
    }
}

fn shift(p: &mut SeqPlaced, dx: f64) {
    for h in &mut p.headers {
        h.rect.x += dx;
        h.x += dx;
    }
    for r in &mut p.rows {
        match r {
            Row::Message { from_x, to_x, label_x, .. } => {
                *from_x += dx;
                *to_x += dx;
                *label_x += dx;
            }
            Row::Note { rect, .. } => rect.x += dx,
        }
    }
    for f in &mut p.frames {
        f.rect.x += dx;
    }
    for a in &mut p.activations {
        a.x += dx;
    }
    for g in &mut p.groups {
        g.rect.x += dx;
    }
}
