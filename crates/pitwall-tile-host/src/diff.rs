//! The host's keyed diff of two frames (runtime.md §2.7, ui.md §3.3): which
//! nodes are new, gone or changed, and for each new node the old node it
//! continues, so the app keeps its GPUI state (scroll, focus, hover) by key
//! path. In the real design this is `pitwall-core`'s `tile::model`.

use std::collections::HashMap;

use crate::{Frame, Node};

/// Tree limits checked before anything is drawn (runtime.md §2.5).
pub const MAX_NODES: usize = 20_000;

#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    /// For each node of the new frame, the index of the node with the same
    /// key path in the old frame, if any.
    pub continues: Vec<Option<u32>>,
    pub added: usize,
    pub removed: usize,
    /// Same key path, different element or style.
    pub changed: usize,
}

/// Why a frame was refused.
#[derive(Debug, PartialEq)]
pub enum Invalid {
    TooManyNodes(usize),
    /// A parent must come before its children; exactly one root, first.
    BadParent(usize),
    DuplicateKey(usize),
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Invalid::TooManyNodes(n) => write!(f, "{n} nodes (at most {MAX_NODES})"),
            Invalid::BadParent(i) => write!(f, "node {i}: bad parent"),
            Invalid::DuplicateKey(i) => write!(f, "node {i}: key used twice under one parent"),
        }
    }
}

impl std::error::Error for Invalid {}

/// Key path hashes of a frame's nodes, checking its shape on the way.
/// A 64-bit FNV-1a of (parent's path hash, key). The spike trusts the hash;
/// the real model compares the keys on a hit.
pub fn paths(frame: &Frame) -> Result<Vec<u64>, Invalid> {
    let nodes = &frame.nodes;
    if nodes.len() > MAX_NODES {
        return Err(Invalid::TooManyNodes(nodes.len()));
    }
    let mut out: Vec<u64> = Vec::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let parent = match (i, n.parent) {
            (0, None) => 0xcbf2_9ce4_8422_2325,
            (i, Some(p)) if i > 0 && (p as usize) < i => out[p as usize],
            _ => return Err(Invalid::BadParent(i)),
        };
        let mut h = parent;
        for b in n.key.as_bytes().iter().chain([0xffu8].iter()) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        out.push(h);
    }
    Ok(out)
}

/// Diff `new` against `old`. `old_paths` is what [`paths`] gave for `old`
/// (kept from the previous frame, so each frame is hashed once).
pub fn diff(old: &Frame, old_paths: &[u64], new: &Frame, new_paths: &[u64]) -> Result<Diff, Invalid> {
    let index: HashMap<u64, u32> =
        old_paths.iter().enumerate().map(|(i, h)| (*h, i as u32)).collect();
    let mut seen = HashMap::with_capacity(new_paths.len());
    let mut d = Diff { continues: Vec::with_capacity(new.nodes.len()), ..Diff::default() };
    let mut matched = 0;
    for (i, (node, h)) in new.nodes.iter().zip(new_paths).enumerate() {
        if seen.insert(*h, ()).is_some() {
            return Err(Invalid::DuplicateKey(i));
        }
        match index.get(h) {
            Some(&o) => {
                matched += 1;
                if !same(&old.nodes[o as usize], node) {
                    d.changed += 1;
                }
                d.continues.push(Some(o));
            }
            None => {
                d.added += 1;
                d.continues.push(None);
            }
        }
    }
    d.removed = old.nodes.len() - matched;
    Ok(d)
}

fn same(a: &Node, b: &Node) -> bool {
    a.element == b.element && a.style == b.style
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Element, Style};

    fn n(parent: Option<u32>, key: &str, text: &str) -> Node {
        Node { parent, key: key.into(), element: Element::Text(text.into()), style: vec![Style::FlexRow] }
    }

    fn frame(nodes: Vec<Node>) -> (Frame, Vec<u64>) {
        let f = Frame { nodes };
        let p = paths(&f).unwrap();
        (f, p)
    }

    #[test]
    fn same_keys_continue_and_changes_are_counted() {
        let (a, pa) = frame(vec![n(None, "root", ""), n(Some(0), "x", "1"), n(Some(0), "y", "2")]);
        let (b, pb) = frame(vec![n(None, "root", ""), n(Some(0), "y", "3"), n(Some(0), "z", "4")]);
        let d = diff(&a, &pa, &b, &pb).unwrap();
        assert_eq!(d.continues, vec![Some(0), Some(2), None]);
        assert_eq!((d.added, d.removed, d.changed), (1, 1, 1));
    }

    #[test]
    fn the_same_key_under_another_parent_is_another_node() {
        let (a, pa) = frame(vec![n(None, "root", ""), n(Some(0), "a", ""), n(Some(1), "k", "")]);
        let (b, pb) = frame(vec![n(None, "root", ""), n(Some(0), "b", ""), n(Some(1), "k", "")]);
        let d = diff(&a, &pa, &b, &pb).unwrap();
        assert_eq!(d.continues, vec![Some(0), None, None]);
    }

    #[test]
    fn bad_shapes_are_refused() {
        let f = Frame { nodes: vec![n(None, "root", ""), n(Some(2), "x", ""), n(Some(0), "y", "")] };
        assert_eq!(paths(&f), Err(Invalid::BadParent(1)));
        let f = Frame { nodes: vec![n(None, "root", ""), n(None, "x", "")] };
        assert_eq!(paths(&f), Err(Invalid::BadParent(1)));
        let (a, pa) = frame(vec![n(None, "root", "")]);
        let (b, pb) = frame(vec![n(None, "root", ""), n(Some(0), "x", ""), n(Some(0), "x", "")]);
        assert_eq!(diff(&a, &pa, &b, &pb), Err(Invalid::DuplicateKey(2)));
    }
}
