//! Checking a frame and diffing it with the one before (ui.md §3.3,
//! runtime.md §2.7). Done on the tile's worker, so the UI thread gets a
//! frame that is known to be well formed together with its diff.
//!
//! Nodes are matched by key path: a node continues an old node when its
//! key equals the old node's and its parent continues the old node's
//! parent (the root continues the old root when their keys match). Exact,
//! with no hashing.

use std::collections::{HashMap, HashSet};

use crate::limits::{MAX_FRAME_BYTES, MAX_INPUT_BYTES, MAX_KEY_BYTES, MAX_NODES};
use crate::ui::{Element, Frame, Icon, Node};

/// How a frame differs from the one before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// For each node of the new frame, the index of the node with the same
    /// key path in the old frame, if any: the app moves that node's state
    /// (scroll position, focus, typed text, hover) to it.
    pub continues: Vec<Option<u32>>,
    /// New nodes whose content (element, styles, listeners, focusable,
    /// tooltip, label) differs from the node they continue, by index.
    pub changed: Vec<u32>,
    /// Nodes of the new frame that continue none.
    pub added: usize,
    /// Nodes of the old frame that nothing continues.
    pub removed: usize,
}

impl Diff {
    /// Nothing changed: same key paths, same content.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added == 0 && self.removed == 0
    }
}

/// Why a frame was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// More than [`MAX_NODES`] nodes.
    TooManyNodes(usize),
    /// About this many bytes, more than [`MAX_FRAME_BYTES`].
    TooLarge(usize),
    /// Node `i`'s parent isn't an earlier node, or a node other than the
    /// first has none, or the first has one.
    BadParent(usize),
    /// Node `i`'s key is used by an earlier sibling.
    DuplicateKey(usize),
    /// Node `i`'s key is longer than [`MAX_KEY_BYTES`].
    LongKey(usize),
    /// Node `i` is a text field whose value is over [`MAX_INPUT_BYTES`].
    LongInput(usize),
    /// Node `i`'s canvas, code view, diff view or drag is over a limit.
    Block(usize, crate::blocks::Problem),
    /// Node `i` is a component (a list, a table, a menu, an image…) that
    /// breaks one of its rules; the text says which.
    Component(usize, String),
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Invalid::TooManyNodes(n) => write!(f, "{n} nodes (at most {MAX_NODES})"),
            Invalid::TooLarge(n) => write!(f, "about {} KiB (at most {} KiB)", n / 1024, MAX_FRAME_BYTES / 1024),
            Invalid::BadParent(i) => write!(f, "node {i} has a bad parent (the root comes first, each node after its parent)"),
            Invalid::DuplicateKey(i) => write!(f, "node {i}'s key is used twice under one parent"),
            Invalid::LongKey(i) => write!(f, "node {i}'s key is longer than {MAX_KEY_BYTES} bytes"),
            Invalid::LongInput(i) => write!(f, "node {i}'s text field holds more than {} KiB", MAX_INPUT_BYTES / 1024),
            Invalid::Block(i, p) => write!(f, "node {i} {p}"),
            Invalid::Component(i, why) => write!(f, "node {i}: {why}"),
        }
    }
}

impl std::error::Error for Invalid {}

/// Check a frame's shape and limits.
pub fn check(frame: &Frame) -> Result<(), Invalid> {
    let nodes = &frame.nodes;
    if nodes.len() > MAX_NODES {
        return Err(Invalid::TooManyNodes(nodes.len()));
    }
    let mut keys: HashSet<(u32, &str)> = HashSet::with_capacity(nodes.len());
    let mut bytes = 64 + frame.focus.as_ref().map_or(0, String::len) + frame.keymap.iter().map(|b| b.keys.len() + 16).sum::<usize>();
    for (i, n) in nodes.iter().enumerate() {
        match (i, n.parent) {
            (0, None) => {}
            (i, Some(p)) if i > 0 && (p as usize) < i => {
                if !keys.insert((p, n.key.as_str())) {
                    return Err(Invalid::DuplicateKey(i));
                }
            }
            _ => return Err(Invalid::BadParent(i)),
        }
        if n.key.len() > MAX_KEY_BYTES {
            return Err(Invalid::LongKey(i));
        }
        if let Element::Input(input) = &n.element {
            if input.value.as_ref().is_some_and(|v| v.len() > MAX_INPUT_BYTES) {
                return Err(Invalid::LongInput(i));
            }
        }
        crate::blocks::check(n).map_err(|p| Invalid::Block(i, p))?;
        crate::components::check(i, n).map_err(|why| Invalid::Component(i, why))?;
        bytes += node_bytes(n);
        if bytes > MAX_FRAME_BYTES {
            return Err(Invalid::TooLarge(bytes));
        }
    }
    Ok(())
}

/// About how many bytes a frame takes through the canonical ABI, as
/// [`check`] counts them against `MAX_FRAME_BYTES`.
pub fn frame_bytes(frame: &Frame) -> usize {
    64 + frame.focus.as_ref().map_or(0, String::len) + frame.keymap.iter().map(|b| b.keys.len() + 16).sum::<usize>() + frame.nodes.iter().map(node_bytes).sum::<usize>()
}

/// About how many bytes a node takes through the canonical ABI.
fn node_bytes(n: &Node) -> usize {
    let icon = |i: &Icon| match i {
        Icon::Named(s) => s.len() + 8,
    };
    let element = match &n.element {
        Element::Box => 0,
        Element::Text(t) => t.len(),
        Element::Button(b) => b.label.len() + b.icon.as_ref().map_or(0, icon),
        Element::Icon(i) => icon(i),
        Element::Scroll(_) => 4,
        Element::Input(i) => i.value.as_ref().map_or(0, String::len) + i.placeholder.as_ref().map_or(0, String::len) + 32,
        e => crate::components::bytes(e),
    } + crate::blocks::bytes(n);
    48 + n.key.len()
        + element
        + n.style.len() * 24
        + n.on.len() * 8
        + n.tooltip.as_ref().map_or(0, String::len)
        + n.label.as_ref().map_or(0, String::len)
}

/// Diff `new` with `old` (`None`: the view's first frame). Both must have
/// passed [`check`].
pub fn diff(old: Option<&Frame>, new: &Frame) -> Diff {
    let empty = Frame { nodes: Vec::new(), focus: None, keymap: Vec::new() };
    let old = old.unwrap_or(&empty);
    let index: HashMap<(u32, &str), u32> = old
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.parent.map(|p| ((p, n.key.as_str()), i as u32)))
        .collect();
    let mut d = Diff { continues: Vec::with_capacity(new.nodes.len()), ..Diff::default() };
    let mut matched = 0;
    for (i, n) in new.nodes.iter().enumerate() {
        let from = match n.parent {
            None => old.nodes.first().filter(|r| r.key == n.key).map(|_| 0),
            Some(p) => d.continues[p as usize].and_then(|op| index.get(&(op, n.key.as_str())).copied()),
        };
        match from {
            Some(o) => {
                matched += 1;
                if !same_content(&old.nodes[o as usize], n) {
                    d.changed.push(i as u32);
                }
            }
            None => d.added += 1,
        }
        d.continues.push(from);
    }
    d.removed = old.nodes.len() - matched;
    d
}

fn same_content(a: &Node, b: &Node) -> bool {
    a.element == b.element && a.style == b.style && a.on == b.on && a.focusable == b.focusable && a.tooltip == b.tooltip && a.label == b.label
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Input, Style};

    fn n(parent: Option<u32>, key: &str, text: &str) -> Node {
        Node { parent, key: key.into(), element: Element::Text(text.into()), style: vec![Style::FlexRow], on: vec![], focusable: false, tooltip: None, label: None }
    }

    fn frame(nodes: Vec<Node>) -> Frame {
        let f = Frame { nodes, focus: None, keymap: vec![] };
        check(&f).unwrap();
        f
    }

    #[test]
    fn same_key_paths_continue_and_changes_are_listed() {
        let a = frame(vec![n(None, "root", ""), n(Some(0), "x", "1"), n(Some(0), "y", "2")]);
        let b = frame(vec![n(None, "root", ""), n(Some(0), "y", "3"), n(Some(0), "z", "4")]);
        let d = diff(Some(&a), &b);
        assert_eq!(d.continues, vec![Some(0), Some(2), None]);
        assert_eq!((d.changed, d.added, d.removed), (vec![1], 1, 1));
        assert!(diff(Some(&b), &b).is_empty());
        let first = diff(None, &b);
        assert_eq!((first.added, first.removed), (3, 0));
    }

    #[test]
    fn the_same_key_under_another_parent_is_another_node() {
        let a = frame(vec![n(None, "root", ""), n(Some(0), "a", ""), n(Some(1), "k", "")]);
        let b = frame(vec![n(None, "root", ""), n(Some(0), "b", ""), n(Some(1), "k", "")]);
        assert_eq!(diff(Some(&a), &b).continues, vec![Some(0), None, None]);
        // Another root key: nothing continues.
        let c = frame(vec![n(None, "other", ""), n(Some(0), "a", "")]);
        assert_eq!(diff(Some(&a), &c).continues, vec![None, None]);
    }

    #[test]
    fn bad_shapes_and_sizes_are_refused() {
        let f = |nodes| Frame { nodes, focus: None, keymap: vec![] };
        assert_eq!(check(&f(vec![n(None, "r", ""), n(Some(2), "x", ""), n(Some(0), "y", "")])), Err(Invalid::BadParent(1)));
        assert_eq!(check(&f(vec![n(None, "r", ""), n(None, "x", "")])), Err(Invalid::BadParent(1)));
        assert_eq!(check(&f(vec![n(Some(0), "r", "")])), Err(Invalid::BadParent(0)));
        assert_eq!(check(&f(vec![n(None, "r", ""), n(Some(0), "x", ""), n(Some(0), "x", "")])), Err(Invalid::DuplicateKey(2)));
        assert_eq!(check(&f(vec![n(None, &"k".repeat(129), "")])), Err(Invalid::LongKey(0)));
        let many: Vec<Node> = std::iter::once(n(None, "r", "")).chain((0..MAX_NODES).map(|i| n(Some(0), &i.to_string(), ""))).collect();
        assert_eq!(check(&f(many)), Err(Invalid::TooManyNodes(MAX_NODES + 1)));
        let big: Vec<Node> = std::iter::once(n(None, "r", "")).chain((0..3).map(|i| n(Some(0), &i.to_string(), &"x".repeat(1 << 20)))).collect();
        assert!(matches!(check(&f(big)), Err(Invalid::TooLarge(_))));
        let input = Input { value: Some("x".repeat(MAX_INPUT_BYTES + 1)), revision: 1, selection: None, placeholder: None, multiline: true, rows: None, disabled: false };
        let node = Node { element: Element::Input(input), ..n(None, "r", "") };
        assert_eq!(check(&f(vec![node])), Err(Invalid::LongInput(0)));
        assert_eq!(check(&f(vec![])), Ok(()), "an empty view is allowed");
    }
}
