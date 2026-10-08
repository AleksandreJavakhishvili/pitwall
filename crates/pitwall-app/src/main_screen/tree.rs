//! The tiling model of a space: a tree of row/col splits whose leaves are
//! panes, each showing at most one agent (a port of `src/layout/tree.ts`).
//! Every function returns a new tree; inputs are never changed. The JSON
//! shape is the one `ui.json` stores, so both apps read each other's layouts.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    /// Side by side.
    Row,
    /// Stacked.
    Col,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// A node of the layout tree (`{type:"pane",id,agentId}` /
/// `{type:"split",id,dir,children,sizes}` in `ui.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    Pane {
        id: String,
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
    },
    Split {
        id: String,
        dir: Dir,
        children: Vec<Node>,
        /// Fractions, one per child, summing to 1.
        sizes: Vec<f64>,
    },
}

/// A leaf of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub id: String,
    pub agent_id: Option<String>,
}

/// Fixed tilings ("3" is 1 big + 2 stacked; "CxR" is C columns × R rows) and
/// the auto grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    One,
    Two,
    Three,
    G2x2,
    G3x2,
    G3x3,
    G4x3,
    G4x4,
    Auto,
}

impl Preset {
    pub const ALL: [Preset; 9] = [
        Preset::One,
        Preset::Two,
        Preset::Three,
        Preset::G2x2,
        Preset::G3x2,
        Preset::G3x3,
        Preset::G4x3,
        Preset::G4x4,
        Preset::Auto,
    ];

    /// Panes it holds (`PRESET_COUNT`; auto: as many as fit).
    pub fn count(self) -> usize {
        match self {
            Preset::One => 1,
            Preset::Two => 2,
            Preset::Three => 3,
            Preset::G2x2 => 4,
            Preset::G3x2 => 6,
            Preset::G3x3 => 9,
            Preset::G4x3 => 12,
            Preset::G4x4 => 16,
            Preset::Auto => usize::MAX,
        }
    }

    /// `PRESET_LABEL`.
    pub fn label(self) -> &'static str {
        match self {
            Preset::One => "1",
            Preset::Two => "2",
            Preset::Three => "1 big + 2",
            Preset::G2x2 => "2×2",
            Preset::G3x2 => "3×2",
            Preset::G3x3 => "3×3",
            Preset::G4x3 => "4×3",
            Preset::G4x4 => "4×4",
            Preset::Auto => "Auto grid",
        }
    }

    /// Columns × rows of a grid preset.
    fn grid(self) -> (usize, usize) {
        match self {
            Preset::One | Preset::Auto => (1, 1),
            Preset::Two => (2, 1),
            Preset::Three => (2, 1),
            Preset::G2x2 => (2, 2),
            Preset::G3x2 => (3, 2),
            Preset::G3x3 => (3, 3),
            Preset::G4x3 => (4, 3),
            Preset::G4x4 => (4, 4),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The smallest tile a density allows, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinSize {
    pub min_w: f64,
    pub min_h: f64,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh id like the React app's (`pane-<time><n>`), unique in this process.
pub fn new_id(prefix: &str) -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}{}", radix36(ms), radix36(n))
}

fn radix36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

impl Node {
    pub fn pane(agent_id: Option<String>) -> Node {
        Node::Pane {
            id: new_id("pane"),
            agent_id,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Node::Pane { id, .. } | Node::Split { id, .. } => id,
        }
    }

    /// Panes in reading order.
    pub fn leaves(&self) -> Vec<Pane> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<Pane>) {
        match self {
            Node::Pane { id, agent_id } => out.push(Pane {
                id: id.clone(),
                agent_id: agent_id.clone(),
            }),
            Node::Split { children, .. } => children.iter().for_each(|c| c.collect(out)),
        }
    }

    pub fn find_pane(&self, pane_id: &str) -> Option<Pane> {
        self.leaves().into_iter().find(|p| p.id == pane_id)
    }

    pub fn find_by_agent(&self, agent_id: &str) -> Option<Pane> {
        self.leaves()
            .into_iter()
            .find(|p| p.agent_id.as_deref() == Some(agent_id))
    }

    /// The agents shown, in reading order.
    pub fn agents(&self) -> Vec<String> {
        self.leaves()
            .into_iter()
            .filter_map(|p| p.agent_id)
            .collect()
    }

    /// The node with this id (a pane or a split).
    pub fn find_node(&self, id: &str) -> Option<&Node> {
        if self.id() == id {
            return Some(self);
        }
        match self {
            Node::Pane { .. } => None,
            Node::Split { children, .. } => children.iter().find_map(|c| c.find_node(id)),
        }
    }

    fn map_panes(&self, f: &mut impl FnMut(&str, &Option<String>) -> Option<String>) -> Node {
        match self {
            Node::Pane { id, agent_id } => Node::Pane {
                id: id.clone(),
                agent_id: f(id, agent_id),
            },
            Node::Split {
                id,
                dir,
                children,
                sizes,
            } => Node::Split {
                id: id.clone(),
                dir: *dir,
                children: children.iter().map(|c| c.map_panes(f)).collect(),
                sizes: sizes.clone(),
            },
        }
    }

    /// Put `agent` in pane `pane_id`; any other pane showing it is emptied.
    pub fn set_pane_agent(&self, pane_id: &str, agent: Option<&str>) -> Node {
        self.map_panes(&mut |id, current| {
            if id == pane_id {
                agent.map(str::to_string)
            } else if agent.is_some() && current.as_deref() == agent {
                None
            } else {
                current.clone()
            }
        })
    }
}

fn normalize_sizes(sizes: &[f64]) -> Vec<f64> {
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 {
        return sizes.iter().map(|_| 1.0 / sizes.len() as f64).collect();
    }
    sizes.iter().map(|s| s / total).collect()
}

/// Collapse one-child splits and merge nested splits of the same direction.
pub fn normalize(node: Option<Node>) -> Option<Node> {
    let node = node?;
    let Node::Split {
        id,
        dir,
        children,
        sizes,
    } = node
    else {
        return Some(node);
    };
    let mut kids = Vec::new();
    let mut fr = Vec::new();
    for (i, c) in children.into_iter().enumerate() {
        let share = sizes.get(i).copied().unwrap_or(0.0);
        match normalize(Some(c)) {
            None => {}
            Some(Node::Split {
                dir: d,
                children: cc,
                sizes: cs,
                ..
            }) if d == dir => {
                for (j, x) in cc.into_iter().enumerate() {
                    kids.push(x);
                    fr.push(share * cs.get(j).copied().unwrap_or(0.0));
                }
            }
            Some(n) => {
                kids.push(n);
                fr.push(share);
            }
        }
    }
    match kids.len() {
        0 => None,
        1 => kids.pop(),
        _ => Some(Node::Split {
            id,
            dir,
            children: kids,
            sizes: normalize_sizes(&fr),
        }),
    }
}

/// Remove a pane; its room goes to its neighbours. `None` if it was the last.
pub fn remove_pane(node: &Node, pane_id: &str) -> Option<Node> {
    fn rec(n: &Node, pane_id: &str) -> Option<Node> {
        match n {
            Node::Pane { id, .. } => (id != pane_id).then(|| n.clone()),
            Node::Split {
                id,
                dir,
                children,
                sizes,
            } => {
                let mut kept = Vec::new();
                let mut fr = Vec::new();
                for (i, c) in children.iter().enumerate() {
                    if let Some(r) = rec(c, pane_id) {
                        kept.push(r);
                        fr.push(sizes.get(i).copied().unwrap_or(0.0));
                    }
                }
                if kept.is_empty() {
                    return None;
                }
                Some(Node::Split {
                    id: id.clone(),
                    dir: *dir,
                    children: kept,
                    sizes: normalize_sizes(&fr),
                })
            }
        }
    }
    normalize(rec(node, pane_id))
}

/// Take an agent out: its pane is removed, or emptied if it is the only one.
pub fn drop_agent(node: &Node, agent_id: &str) -> Node {
    let Some(pane) = node.find_by_agent(agent_id) else {
        return node.clone();
    };
    if node.leaves().len() == 1 {
        return node.set_pane_agent(&pane.id, None);
    }
    remove_pane(node, &pane.id).unwrap_or_else(|| Node::pane(None))
}

/// Split pane `pane_id`, adding a pane with `agent` on `side`. The agent
/// leaves any other pane first (an agent lives in one pane). Returns the new
/// tree and the pane now showing the agent.
pub fn split_pane(node: &Node, pane_id: &str, side: Side, agent: Option<&str>) -> (Node, String) {
    let mut tree = node.clone();
    if let Some(a) = agent {
        if let Some(holder) = tree.find_by_agent(a) {
            if holder.id == pane_id {
                return (tree, pane_id.to_string());
            }
            tree = remove_pane(&tree, &holder.id).unwrap_or_else(|| Node::pane(None));
            if tree.find_pane(pane_id).is_none() {
                let only = tree.leaves()[0].id.clone();
                return (tree.set_pane_agent(&only, Some(a)), only);
            }
        }
    }
    let dir = match side {
        Side::Left | Side::Right => Dir::Row,
        Side::Top | Side::Bottom => Dir::Col,
    };
    let before = matches!(side, Side::Left | Side::Top);
    let fresh_id = new_id("pane");
    let fresh = Node::Pane {
        id: fresh_id.clone(),
        agent_id: agent.map(str::to_string),
    };
    fn rec(n: &Node, pane_id: &str, dir: Dir, before: bool, fresh: &Node) -> Node {
        match n {
            Node::Pane { id, .. } => {
                if id != pane_id {
                    return n.clone();
                }
                let children = if before {
                    vec![fresh.clone(), n.clone()]
                } else {
                    vec![n.clone(), fresh.clone()]
                };
                Node::Split {
                    id: new_id("split"),
                    dir,
                    children,
                    sizes: vec![0.5, 0.5],
                }
            }
            Node::Split {
                id,
                dir: d,
                children,
                sizes,
            } => {
                let idx = children
                    .iter()
                    .position(|c| matches!(c, Node::Pane { id, .. } if id == pane_id));
                if let (Some(idx), true) = (idx, *d == dir) {
                    let half = sizes[idx] / 2.0;
                    let mut kids = children.clone();
                    let mut fr = sizes.clone();
                    kids.insert(if before { idx } else { idx + 1 }, fresh.clone());
                    fr.splice(idx..=idx, [half, half]);
                    return Node::Split {
                        id: id.clone(),
                        dir: *d,
                        children: kids,
                        sizes: fr,
                    };
                }
                Node::Split {
                    id: id.clone(),
                    dir: *d,
                    children: children
                        .iter()
                        .map(|c| rec(c, pane_id, dir, before, fresh))
                        .collect(),
                    sizes: sizes.clone(),
                }
            }
        }
    }
    let out = normalize(Some(rec(&tree, pane_id, dir, before, &fresh))).unwrap_or(tree);
    (out, fresh_id)
}

/// Replace one split's sizes (committing a divider drag).
pub fn set_split_sizes(node: &Node, split_id: &str, new: &[f64]) -> Node {
    match node {
        Node::Pane { .. } => node.clone(),
        Node::Split {
            id,
            dir,
            children,
            sizes,
        } => {
            if id == split_id && new.len() == children.len() {
                return Node::Split {
                    id: id.clone(),
                    dir: *dir,
                    children: children.clone(),
                    sizes: normalize_sizes(new),
                };
            }
            Node::Split {
                id: id.clone(),
                dir: *dir,
                children: children
                    .iter()
                    .map(|c| set_split_sizes(c, split_id, new))
                    .collect(),
                sizes: sizes.clone(),
            }
        }
    }
}

/// Move the divider between child `index` and `index + 1` by `delta`
/// (a fraction), keeping each side at least `min_frac`.
pub fn resized(sizes: &[f64], index: usize, delta: f64, min_frac: f64) -> Vec<f64> {
    let mut out = sizes.to_vec();
    let (Some(&a), Some(&b)) = (sizes.get(index), sizes.get(index + 1)) else {
        return out;
    };
    let d = delta.min(b - min_frac).max(min_frac - a);
    out[index] = a + d;
    out[index + 1] = b - d;
    out
}

/// A grid of rows, `rows[i]` panes side by side in row i, filled with
/// `agents` in reading order (missing slots are empty panes).
pub fn build_grid(rows: &[usize], agents: &[String]) -> Node {
    let mut next = 0;
    let mut pane = || {
        let a = agents.get(next).cloned();
        next += 1;
        Node::pane(a)
    };
    let split = |dir: Dir, mut children: Vec<Node>| {
        if children.len() == 1 {
            return children.pop().expect("one child");
        }
        let n = children.len();
        Node::Split {
            id: new_id("split"),
            dir,
            children,
            sizes: vec![1.0 / n as f64; n],
        }
    };
    let counts: Vec<usize> = rows.iter().copied().filter(|&n| n > 0).collect();
    if counts.is_empty() {
        return pane();
    }
    let row_nodes: Vec<Node> = counts
        .iter()
        .map(|&n| split(Dir::Row, (0..n).map(|_| pane()).collect()))
        .collect();
    split(Dir::Col, row_nodes)
}

/// A preset layout filled with `agents` in order.
pub fn build_preset(preset: Preset, agents: &[String]) -> Node {
    if preset == Preset::Three {
        let p = |i: usize| Node::pane(agents.get(i).cloned());
        let col = Node::Split {
            id: new_id("split"),
            dir: Dir::Col,
            children: vec![p(1), p(2)],
            sizes: vec![0.5, 0.5],
        };
        return Node::Split {
            id: new_id("split"),
            dir: Dir::Row,
            children: vec![p(0), col],
            sizes: vec![0.6, 0.4],
        };
    }
    let (cols, rows) = preset.grid();
    build_grid(&vec![cols; rows], agents)
}

/// The smallest preset that holds `n` agents.
pub fn preset_for(n: usize) -> Preset {
    match n {
        0 | 1 => Preset::One,
        2 => Preset::Two,
        3 => Preset::Three,
        4 => Preset::G2x2,
        _ => Preset::G3x2,
    }
}

/// `n` panes over `rows` rows as evenly as possible (longer rows first).
pub fn even_rows(n: usize, rows: usize) -> Vec<usize> {
    let r = rows.min(n).max(1);
    let base = n / r;
    let extra = n % r;
    (0..r).map(|i| base + usize::from(i < extra)).collect()
}

/// Auto grid: row counts that tile as many of `n` agents as fit at `min`,
/// preferring at most `max_per_row` columns and then the biggest tiles.
pub fn auto_grid(n: usize, area: (f64, f64), min: MinSize, max_per_row: usize) -> Vec<usize> {
    if n <= 1 {
        return vec![1];
    }
    let max_cols = ((area.0 / min.min_w).floor() as usize).max(1);
    let max_rows = ((area.1 / min.min_h).floor() as usize).max(1);
    let k = n.min(max_cols * max_rows);
    let mut best: Option<(usize, bool, f64)> = None;
    for rows in 1..=k.min(max_rows) {
        let cols = k.div_ceil(rows);
        if cols > max_cols || (rows - 1) * cols >= k {
            continue;
        }
        let over = cols > max_per_row.max(1);
        let scale = (area.0 / cols as f64 / min.min_w).min(area.1 / rows as f64 / min.min_h);
        let better = match best {
            None => true,
            Some((_, b_over, b_scale)) => {
                (b_over && !over) || (b_over == over && scale > b_scale + 1e-9)
            }
        };
        if better {
            best = Some((rows, over, scale));
        }
    }
    even_rows(k, best.map(|b| b.0).unwrap_or(1))
}

/// Pixel rects of every pane inside `rect` (dividers ignored).
pub fn pane_rects(node: &Node, rect: Rect) -> Vec<(String, Rect)> {
    fn rec(n: &Node, r: Rect, out: &mut Vec<(String, Rect)>) {
        match n {
            Node::Pane { id, .. } => out.push((id.clone(), r)),
            Node::Split {
                dir,
                children,
                sizes,
                ..
            } => {
                let mut offset = 0.0;
                for (i, c) in children.iter().enumerate() {
                    let f = sizes.get(i).copied().unwrap_or(0.0);
                    let sub = match dir {
                        Dir::Row => Rect {
                            x: r.x + offset * r.w,
                            y: r.y,
                            w: f * r.w,
                            h: r.h,
                        },
                        Dir::Col => Rect {
                            x: r.x,
                            y: r.y + offset * r.h,
                            w: r.w,
                            h: f * r.h,
                        },
                    };
                    rec(c, sub, out);
                    offset += f;
                }
            }
        }
    }
    let mut out = Vec::new();
    rec(node, rect, &mut out);
    out
}

/// Whether every pane of `preset` is at least `min` in `area`.
pub fn preset_fits(preset: Preset, area: (f64, f64), min: MinSize) -> bool {
    if matches!(preset, Preset::One | Preset::Auto) {
        return true;
    }
    let tree = build_preset(preset, &[]);
    pane_rects(
        &tree,
        Rect {
            x: 0.0,
            y: 0.0,
            w: area.0,
            h: area.1,
        },
    )
    .iter()
    .all(|(_, r)| r.w >= min.min_w && r.h >= min.min_h)
}

/// Presets offered for a space of `area` at a minimum tile size.
pub fn available_presets(area: Option<(f64, f64)>, min: MinSize) -> Vec<Preset> {
    match area {
        None => Preset::ALL.to_vec(),
        Some(a) => Preset::ALL
            .into_iter()
            .filter(|p| preset_fits(*p, a, min))
            .collect(),
    }
}

/// Render-time fit: fold panes smaller than `min` (the last offender in
/// reading order first, empty ones before filled ones, `keep` last) until
/// everything fits; one pane always stays. Returns the tree to draw and the
/// folded panes.
pub fn fit_layout(
    node: &Node,
    area: (f64, f64),
    min: MinSize,
    keep: Option<&str>,
) -> (Node, Vec<Pane>) {
    let mut tree = node.clone();
    let mut hidden = Vec::new();
    loop {
        let ls = tree.leaves();
        if ls.len() <= 1 {
            break;
        }
        let rects: HashMap<String, Rect> = pane_rects(
            &tree,
            Rect {
                x: 0.0,
                y: 0.0,
                w: area.0,
                h: area.1,
            },
        )
        .into_iter()
        .collect();
        let bad: Vec<&Pane> = ls
            .iter()
            .filter(|p| {
                rects
                    .get(&p.id)
                    .is_some_and(|r| r.w < min.min_w || r.h < min.min_h)
            })
            .collect();
        if bad.is_empty() {
            break;
        }
        let order: Vec<&Pane> = bad.into_iter().rev().collect();
        let victim = order
            .iter()
            .find(|p| Some(p.id.as_str()) != keep && p.agent_id.is_none())
            .or_else(|| order.iter().find(|p| Some(p.id.as_str()) != keep))
            .unwrap_or(&order[0]);
        let victim = (*victim).clone();
        match remove_pane(&tree, &victim.id) {
            Some(t) => tree = t,
            None => break,
        }
        hidden.push(victim);
    }
    (tree, hidden)
}

/// Which drop zone a point (0..1 inside a pane) falls in.
pub fn drop_zone(fx: f64, fy: f64) -> Option<Side> {
    const EDGE: f64 = 0.28;
    let d = [
        (Side::Left, fx),
        (Side::Right, 1.0 - fx),
        (Side::Top, fy),
        (Side::Bottom, 1.0 - fy),
    ];
    let (side, dist) = d
        .into_iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .expect("four sides");
    (dist < EDGE).then_some(side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn presets_hold_their_count_and_round_trip_as_json() {
        for p in Preset::ALL.into_iter().filter(|p| *p != Preset::Auto) {
            assert_eq!(build_preset(p, &[]).leaves().len(), p.count(), "{p:?}");
        }
        let t = build_preset(Preset::Three, &ids(&["a", "b", "c"]));
        assert_eq!(t.agents(), ids(&["a", "b", "c"]));
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["type"], "split");
        assert_eq!(json["dir"], "row");
        assert_eq!(json["children"][0]["agentId"], "a");
        let back: Node = serde_json::from_value(json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn split_and_remove_keep_one_pane_per_agent() {
        let t = Node::pane(Some("a".into()));
        let first = t.leaves()[0].id.clone();
        let (t, b) = split_pane(&t, &first, Side::Right, Some("b"));
        assert_eq!(t.agents(), ids(&["a", "b"]));
        let (t, _) = split_pane(&t, &b, Side::Right, Some("c"));
        // Same direction: siblings, not nested.
        assert!(matches!(&t, Node::Split { children, .. } if children.len() == 3));
        // Moving "a" next to "c" removes its old pane.
        let c = t.find_by_agent("c").unwrap().id;
        let (t, _) = split_pane(&t, &c, Side::Bottom, Some("a"));
        assert_eq!(t.leaves().len(), 3);
        assert_eq!(t.agents().iter().filter(|a| *a == "a").count(), 1);
        let t = drop_agent(&t, "b");
        assert_eq!(t.leaves().len(), 2);
        let only = drop_agent(&drop_agent(&t, "a"), "c");
        assert_eq!(
            only.leaves().len(),
            1,
            "the last pane is emptied, not removed"
        );
        assert!(only.agents().is_empty());
    }

    #[test]
    fn sizes_are_clamped_and_normalised() {
        let r = resized(&[0.5, 0.5], 0, 0.9, 0.08);
        assert!((r[0] - 0.92).abs() < 1e-9 && (r[1] - 0.08).abs() < 1e-9);
        let t = build_preset(Preset::Two, &[]);
        let sid = t.id().to_string();
        let t = set_split_sizes(&t, &sid, &[3.0, 1.0]);
        assert!(matches!(t, Node::Split { ref sizes, .. } if (sizes[0] - 0.75).abs() < 1e-9));
    }

    #[test]
    fn auto_grid_and_fit() {
        let min = MinSize {
            min_w: 400.0,
            min_h: 200.0,
        };
        assert_eq!(auto_grid(4, (1600.0, 900.0), min, 2), vec![2, 2]);
        assert_eq!(auto_grid(1, (1600.0, 900.0), min, 2), vec![1]);
        assert_eq!(even_rows(5, 2), vec![3, 2]);
        let t = build_preset(Preset::G4x4, &ids(&["a", "b"]));
        let keep = t.leaves()[0].id.clone();
        let (fit, hidden) = fit_layout(&t, (900.0, 500.0), min, Some(&keep));
        assert!(fit.leaves().len() <= 4 && !hidden.is_empty());
        assert!(fit.find_pane(&keep).is_some(), "the focused pane stays");
        assert!(available_presets(Some((900.0, 500.0)), min).contains(&Preset::G2x2));
        assert!(!available_presets(Some((900.0, 500.0)), min).contains(&Preset::G4x4));
    }

    #[test]
    fn drop_zones_by_edge() {
        assert_eq!(drop_zone(0.1, 0.5), Some(Side::Left));
        assert_eq!(drop_zone(0.5, 0.95), Some(Side::Bottom));
        assert_eq!(drop_zone(0.5, 0.5), None);
    }
}
