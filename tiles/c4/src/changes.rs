//! The Changes view model (FR-17): the after diagram, with what was added,
//! changed and removed against the before diagram.

use std::collections::HashMap;

use crate::c4::{name_key, Structure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Only in after.
    Added,
    /// In both, with a `new` or `changed` class in after, or a different
    /// Kind, purpose or boundary; for arrows, a different line or heads.
    Changed,
    /// In both, the same.
    Kept,
}

/// The Changes view: one change per box and arrow of the after diagram (by
/// index), plus the before diagram's boxes and arrows that are gone, drawn
/// as ghosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeView {
    pub boxes: Vec<Change>,
    pub arrows: Vec<Change>,
    /// Indices of before's boxes with no box of the same Name in after.
    pub removed_boxes: Vec<usize>,
    /// Indices of before's arrows with no match in after.
    pub removed_arrows: Vec<usize>,
}

/// A box's key: its Name key.
fn box_key(s: &Structure, i: usize) -> String {
    name_key(&s.boxes[i].name)
}

/// An arrow's key: both ends' Name keys and the label.
fn arrow_key(s: &Structure, i: usize) -> (String, String, String) {
    let a = &s.arrows[i];
    let label = a.label.as_deref().unwrap_or("");
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    (name_key(s.end_name(a.from)), name_key(s.end_name(a.to)), label)
}

fn boundary_name(s: &Structure, i: usize) -> Option<String> {
    s.boxes[i].boundary.map(|b| name_key(&s.boundaries[b].name))
}

/// Compares a before/after pair.
pub fn compare(before: &Structure, after: &Structure) -> ChangeView {
    let before_boxes: HashMap<String, usize> =
        (0..before.boxes.len()).map(|i| (box_key(before, i), i)).collect();
    let after_boxes: HashMap<String, usize> =
        (0..after.boxes.len()).map(|i| (box_key(after, i), i)).collect();
    let boxes = (0..after.boxes.len())
        .map(|i| {
            let Some(&j) = before_boxes.get(&box_key(after, i)) else {
                return Change::Added;
            };
            let (a, b) = (&after.boxes[i], &before.boxes[j]);
            let differs = a.kind != b.kind
                || a.purpose != b.purpose
                || boundary_name(after, i) != boundary_name(before, j);
            if a.marks.new || a.marks.changed || differs {
                Change::Changed
            } else {
                Change::Kept
            }
        })
        .collect();
    let removed_boxes = (0..before.boxes.len())
        .filter(|&j| !after_boxes.contains_key(&box_key(before, j)))
        .collect();

    let before_arrows: HashMap<_, usize> =
        (0..before.arrows.len()).map(|i| (arrow_key(before, i), i)).collect();
    let after_arrows: HashMap<_, usize> =
        (0..after.arrows.len()).map(|i| (arrow_key(after, i), i)).collect();
    let arrows = (0..after.arrows.len())
        .map(|i| match before_arrows.get(&arrow_key(after, i)) {
            None => Change::Added,
            Some(&j) => {
                let (a, b) = (&after.arrows[i], &before.arrows[j]);
                let same = a.style == b.style
                    && a.start_head == b.start_head
                    && a.end_head == b.end_head;
                if same {
                    Change::Kept
                } else {
                    Change::Changed
                }
            }
        })
        .collect();
    let removed_arrows = (0..before.arrows.len())
        .filter(|&j| !after_arrows.contains_key(&arrow_key(before, j)))
        .collect();
    ChangeView { boxes, arrows, removed_boxes, removed_arrows }
}
