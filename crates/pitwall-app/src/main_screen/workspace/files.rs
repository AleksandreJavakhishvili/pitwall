//! Files tiles in a space's layout: an agent's file viewer placed among
//! the terminal tiles, wherever the user (or the agent, asked by the user)
//! puts it. One files tile per agent across every space: placing it again
//! moves it. Web tiles (`web.rs`) are placed by the same code ([`Content`]).
//! Pure, as the rest of [`super`].

use serde::{Deserialize, Serialize};

use super::{auto_spot, empty_layout, folded, Fit, UiState};
use crate::main_screen::tree::{self, Content, Node, Side};

/// Where a files tile goes.
#[derive(Debug, Clone, PartialEq)]
pub enum FilesSpot {
    /// On `side` of tile `pane` (splitting it).
    Tile { pane: String, side: Side },
    /// Along `side` of the whole main area: a column or row of its own.
    Edge(Side),
    /// In empty tile `pane`.
    Take(String),
    /// In tile `pane`'s place, as big and where it is: a terminal's tile
    /// (the terminal leaves the layout as when its tile is closed: the
    /// agent keeps running and stays in the sidebar), an empty tile (as
    /// [`FilesSpot::Take`]), or another agent's files tile in the same
    /// space (the two swap places).
    Replace(String),
    /// A tile of its own wherever there is most room (an empty tile first).
    Auto,
}

/// What a files tile has open, kept for the next start.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockedFiles {
    /// What it reads: the agent's id, or a folder's `path:` key.
    pub key: String,
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

/// A file viewer window, kept to open again after a restart: what it has
/// open (as a files tile's), the folders open in its tree, and its own
/// place and size. Unsaved edits are not kept: its files are read again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesWindow {
    #[serde(flatten)]
    pub open: DockedFiles,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
}

/// A files tile's share of what it splits, at first.
pub const FILES_SHARE: f64 = 0.45;
/// Along an edge of the main area.
pub const EDGE_SHARE: f64 = 0.35;

impl UiState {
    /// The space and tile showing agent `owner`'s files.
    pub fn files_tile(&self, owner: &str) -> Option<(String, String)> {
        self.tile_of(&Content::Files(owner.to_string()))
    }

    /// The space and tile showing `c`.
    pub fn tile_of(&self, c: &Content) -> Option<(String, String)> {
        self.spaces
            .iter()
            .find_map(|sp| sp.layout.find_content(c).map(|p| (sp.id.clone(), p.id)))
    }

    /// Agent `owner`'s files tile taken out of every space (what it had
    /// open is kept until [`UiState::forget_files`]).
    pub fn undock_files(self, owner: &str) -> UiState {
        self.undock(&Content::Files(owner.to_string()))
    }

    /// The tile showing `c` taken out of every space.
    pub fn undock(mut self, c: &Content) -> UiState {
        for sp in &mut self.spaces {
            let Some(p) = sp.layout.find_content(c) else { continue };
            sp.layout = tree::remove_pane(&sp.layout, &p.id).unwrap_or_else(empty_layout);
            if sp.maximized_pane_id.as_deref() == Some(p.id.as_str()) {
                sp.maximized_pane_id = None;
            }
            *sp = sp.clone().tidy();
        }
        self
    }

    /// Put agent `owner`'s files tile at `spot` in space `space_id`
    /// (moved there when it was elsewhere), taking `share` of what it
    /// splits. The keyboard's tile stays focused. Returns the tile, or
    /// `None` (nothing changed) when `spot` isn't in that space or is the
    /// tile itself.
    pub fn dock_files(self, space_id: &str, owner: &str, spot: &FilesSpot, share: f64, fit: Option<Fit>) -> (UiState, Option<String>) {
        self.dock(space_id, &Content::Files(owner.to_string()), spot, share, fit)
    }

    /// [`UiState::dock_files`] for any tile that isn't a terminal (files or
    /// a web page): one tile per `c` across every space.
    pub fn dock(self, space_id: &str, c: &Content, spot: &FilesSpot, share: f64, fit: Option<Fit>) -> (UiState, Option<String>) {
        let Some(sp) = self.space(space_id) else { return (self, None) };
        let own = sp.layout.find_content(c).map(|p| p.id);
        let target_ok = match spot {
            FilesSpot::Tile { pane, .. } | FilesSpot::Take(pane) | FilesSpot::Replace(pane) => {
                own.as_deref() != Some(pane.as_str()) && sp.layout.find_pane(pane).is_some()
            }
            FilesSpot::Edge(_) | FilesSpot::Auto => true,
        };
        if !target_ok {
            return (self, None);
        }
        if let FilesSpot::Take(pane) = spot {
            if !sp.layout.find_pane(pane).is_some_and(|p| p.is_empty()) {
                return (self, None);
            }
        }
        if let FilesSpot::Replace(pane) = spot {
            let target = sp.layout.find_pane(pane);
            if let Some(other) = target.as_ref().and_then(|p| p.content()) {
                // Another files or web tile: they swap places, when both
                // are in this space (else nothing: the other would have
                // nowhere to go).
                let Some(mine) = own else { return (self, None) };
                let theirs = pane.clone();
                let next = self.map_space(space_id, |mut sp| {
                    sp.layout = show_content(&sp.layout, &mine, &other);
                    sp.layout = show_content(&sp.layout, &theirs, c);
                    sp.maximized_pane_id = None;
                    sp.tidy()
                });
                return (next, Some(theirs));
            }
        }
        let next = self.undock(c);
        let fresh = Node::content_pane(c);
        let id = fresh.id().to_string();
        let mut placed = Some(id.clone());
        let next = next.map_space(space_id, |mut sp| {
            let only_empty = sp.layout.leaves().len() == 1 && sp.layout.leaves()[0].is_empty();
            let before = sp.layout.clone();
            sp.layout = match spot {
                FilesSpot::Tile { pane, side } => tree::split_with(&sp.layout, pane, *side, &fresh, share),
                FilesSpot::Edge(_) if only_empty => fresh.clone(),
                FilesSpot::Edge(side) => tree::wrap_edge(&sp.layout, *side, &fresh, share.min(EDGE_SHARE)),
                FilesSpot::Take(pane) | FilesSpot::Replace(pane) => {
                    placed = Some(pane.clone());
                    // A terminal it replaces stays one of the space's
                    // agents (shown again from its chip or the sidebar).
                    if let Some(gone) = sp.layout.find_pane(pane).and_then(|p| p.agent_id) {
                        if sp.kind == super::SpaceKind::Custom && !sp.members.contains(&gone) {
                            sp.members.push(gone);
                        }
                    }
                    show_content(&sp.layout, pane, c)
                }
                FilesSpot::Auto => match sp.layout.leaves().into_iter().find(|p| p.is_empty()) {
                    Some(empty) => {
                        placed = Some(empty.id.clone());
                        show_content(&sp.layout, &empty.id, c)
                    }
                    None => {
                        let (at, side) = auto_spot(&sp.layout, fit);
                        tree::split_with(&sp.layout, &at, side, &fresh, 0.5)
                    }
                },
            };
            // Tiles it squeezed until they fold: its row (or column) shared
            // evenly instead, when that folds fewer. (In a tile's place:
            // its size, as it was.)
            if let Some(tile) = placed.as_deref().filter(|_| !matches!(spot, FilesSpot::Replace(_))) {
                sp.layout = roomier(&before, sp.layout, tile, fit);
            }
            sp.maximized_pane_id = None;
            sp.tidy()
        });
        (next, placed)
    }

    /// What agent `owner`'s files tile has open, kept.
    pub fn keep_files(mut self, owner: &str, open: DockedFiles) -> UiState {
        self.docked_files.insert(owner.to_string(), open);
        self
    }

    /// Forget what agent `owner`'s files tile had open (it closed).
    pub fn forget_files(mut self, owner: &str) -> UiState {
        self.docked_files.remove(owner);
        self
    }

    /// Agent `owner`'s file viewer window, kept to open again.
    pub fn keep_files_window(mut self, owner: &str, kept: FilesWindow) -> UiState {
        self.files_windows.insert(owner.to_string(), kept);
        self
    }

    /// Agent `owner`'s file viewer window closed for good.
    pub fn forget_files_window(mut self, owner: &str) -> UiState {
        self.files_windows.remove(owner);
        self
    }
}

/// `after` (tile `tile` just placed), or with the split holding the tile
/// shared evenly when that folds fewer tiles at `fit`.
fn roomier(before: &Node, after: Node, tile: &str, fit: Option<Fit>) -> Node {
    let count = |n: &Node| folded(n, fit, None).len();
    let was = count(before);
    let now = count(&after);
    if fit.is_none() || now <= was {
        return after;
    }
    let Some((split, n)) = parent_split(&after, tile) else { return after };
    let even = tree::set_split_sizes(&after, &split, &vec![1.0; n]);
    if count(&even) < now {
        even
    } else {
        after
    }
}

/// The split holding `id` directly, and how many children it has.
fn parent_split(n: &Node, id: &str) -> Option<(String, usize)> {
    let Node::Split { id: sid, children, .. } = n else { return None };
    if children.iter().any(|c| c.id() == id) {
        return Some((sid.clone(), children.len()));
    }
    children.iter().find_map(|c| parent_split(c, id))
}

/// `layout` with tile `pane` showing `c` (whatever it showed leaves the
/// layout; the tile keeps its id, place and size).
fn show_content(layout: &Node, pane: &str, c: &Content) -> Node {
    fn rec(n: &Node, pane: &str, c: &Content) -> Node {
        match n {
            Node::Pane { id, .. } if id == pane => n.clone().showing(c),
            Node::Pane { .. } => n.clone(),
            Node::Split { id, dir, children, sizes } => Node::Split {
                id: id.clone(),
                dir: *dir,
                children: children.iter().map(|ch| rec(ch, pane, c)).collect(),
                sizes: sizes.clone(),
            },
        }
    }
    rec(layout, pane, c)
}

/// `new` (a layout rebuilt from its agents: a preset) with the files and
/// web tiles of `old` it lost, each a column of its own on the right.
pub fn keep_files_tiles(old: &Node, new: Node) -> Node {
    let mut out = new;
    for c in old.contents() {
        if out.find_content(&c).is_none() {
            out = tree::wrap_edge(&out, Side::Right, &Node::content_pane(&c), EDGE_SHARE);
        }
    }
    out
}

/// Where a files tile is, as the user would say it (the tool's answer,
/// the drop preview, Move… all use [`whereabouts`] and [`Whereabouts::words`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Whereabouts {
    /// On that side of an agent's terminal.
    Next(Side, String),
    /// Between two terminals: the left (or upper) one first.
    Between(String, String),
    /// A column or row of its own along that edge of the space.
    Edge(Side),
    /// No terminal beside it.
    Alone,
}

impl Whereabouts {
    /// In words, with agents' names from `name`: "right of api's terminal",
    /// "a column on the right of the space".
    pub fn words(&self, name: &dyn Fn(&str) -> String) -> String {
        match self {
            Whereabouts::Next(side, a) => {
                let rel = match side {
                    Side::Right => "right of",
                    Side::Left => "left of",
                    Side::Top => "above",
                    Side::Bottom => "below",
                };
                format!("{rel} {}'s terminal", name(a))
            }
            Whereabouts::Between(a, b) => format!("between {}'s and {}'s terminals", name(a), name(b)),
            Whereabouts::Edge(Side::Right) => "a column on the right of the space".into(),
            Whereabouts::Edge(Side::Left) => "a column on the left of the space".into(),
            Whereabouts::Edge(Side::Top) => "a row along the top of the space".into(),
            Whereabouts::Edge(Side::Bottom) => "a row along the bottom of the space".into(),
            Whereabouts::Alone => "on its own in the layout".into(),
        }
    }
}

/// Where tile `pane` of `layout` is, from the tiles as they are laid out
/// (never their order in the tree): next to `anchor`'s terminal when it was
/// placed against it and still touches it; else a column or row along an
/// edge of the space (unless one terminal has all of its inner side);
/// else between two terminals that each have all of one side of it; else
/// next to the terminal it shares most of an edge with.
pub fn whereabouts(layout: &Node, pane: &str, anchor: Option<&str>) -> Whereabouts {
    const E: f64 = 1e-6;
    let rects = tree::pane_rects(layout, tree::Rect { x: 0., y: 0., w: 1., h: 1. });
    let Some(me) = rects.iter().find(|(id, _)| id == pane).map(|(_, r)| *r) else { return Whereabouts::Alone };
    let leaves = layout.leaves();
    let terminal = |id: &str| leaves.iter().find(|p| p.id == id && p.content().is_none()).and_then(|p| p.agent_id.clone());
    // Every tile sharing an edge with it: (tile, the side of that tile it
    // is on, how long the shared edge is).
    let touching: Vec<(&str, Side, f64)> = rects
        .iter()
        .filter(|(id, _)| id != pane)
        .filter_map(|(id, r)| {
            let along_y = (me.y + me.h).min(r.y + r.h) - me.y.max(r.y);
            let along_x = (me.x + me.w).min(r.x + r.w) - me.x.max(r.x);
            let side = if (me.x - (r.x + r.w)).abs() < E && along_y > E {
                (Side::Right, along_y)
            } else if (me.x + me.w - r.x).abs() < E && along_y > E {
                (Side::Left, along_y)
            } else if (me.y - (r.y + r.h)).abs() < E && along_x > E {
                (Side::Bottom, along_x)
            } else if (me.y + me.h - r.y).abs() < E && along_x > E {
                (Side::Top, along_x)
            } else {
                return None;
            };
            Some((id.as_str(), side.0, side.1))
        })
        .collect();
    let terminals: Vec<(String, Side, f64)> = touching.iter().filter_map(|(id, s, l)| Some((terminal(id)?, *s, *l))).collect();
    if let Some((a, s, _)) = anchor.and_then(|a| terminals.iter().find(|(t, _, _)| t == a)) {
        return Whereabouts::Next(*s, a.clone());
    }
    // Along an edge, all the way: a column or row of its own, unless one
    // tile has all of its inner side (then it is that one's neighbour).
    let edges = [
        (Side::Right, (me.x + me.w - 1.).abs() < E && me.h > 1. - E),
        (Side::Left, me.x.abs() < E && me.h > 1. - E),
        (Side::Bottom, (me.y + me.h - 1.).abs() < E && me.w > 1. - E),
        (Side::Top, me.y.abs() < E && me.w > 1. - E),
    ];
    for (side, along) in edges {
        if along && touching.iter().filter(|(_, s, _)| *s == side).count() > 1 {
            return Whereabouts::Edge(side);
        }
    }
    // A terminal with all of one side of it, on the side `of` of it.
    let whole = |of: Side| {
        let len = if matches!(of, Side::Left | Side::Right) { me.h } else { me.w };
        terminals.iter().find(|(_, s, l)| *s == of && (l - len).abs() < E).map(|(t, _, _)| t.clone())
    };
    if let (Some(a), Some(b)) = (whole(Side::Right), whole(Side::Left)) {
        return Whereabouts::Between(a, b);
    }
    if let (Some(a), Some(b)) = (whole(Side::Bottom), whole(Side::Top)) {
        return Whereabouts::Between(a, b);
    }
    // The terminal it shares most of an edge with (beside before above).
    let rank = |s: Side| match s {
        Side::Right => 0,
        Side::Left => 1,
        Side::Bottom => 2,
        Side::Top => 3,
    };
    terminals
        .iter()
        .min_by(|a, b| b.2.total_cmp(&a.2).then(rank(a.1).cmp(&rank(b.1))))
        .map_or(Whereabouts::Alone, |(t, s, _)| Whereabouts::Next(*s, t.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_screen::tree::{build_grid, build_preset, Dir, Preset};
    use crate::main_screen::workspace::{SpaceKind, MAIN};

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A state with space "s" laid out as `layout`, and an empty space "t".
    fn state(layout: Node) -> UiState {
        let mut s = UiState::default();
        let mut sp = s.spaces[0].clone();
        sp.id = "s".into();
        sp.kind = SpaceKind::Custom;
        sp.layout = layout;
        let mut t = sp.clone();
        t.id = "t".into();
        t.layout = Node::pane(None);
        s.spaces = vec![sp, t];
        let _ = MAIN;
        s
    }

    fn pane_of(s: &UiState, agent: &str) -> String {
        s.space("s").unwrap().layout.find_by_agent(agent).unwrap().id
    }

    #[test]
    fn a_files_tile_splits_a_terminal_tile_on_each_side() {
        for (side, dir, first) in [
            (Side::Right, Dir::Row, false),
            (Side::Left, Dir::Row, true),
            (Side::Top, Dir::Col, true),
            (Side::Bottom, Dir::Col, false),
        ] {
            let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
            let b = pane_of(&s, "b");
            let (s, tile) = s.dock_files("s", "a", &FilesSpot::Tile { pane: b.clone(), side }, 0.4, None);
            let layout = &s.space("s").unwrap().layout;
            let tile = tile.unwrap();
            assert_eq!(layout.find_files("a").unwrap().id, tile);
            // Both terminals stay; the files tile sits on that side of b.
            assert_eq!(layout.agents(), ids(&["a", "b"]));
            let Node::Split { children, .. } = layout else { panic!() };
            let holder = children.iter().find(|c| c.find_node(&tile).is_some()).unwrap();
            match holder {
                Node::Split { dir: d, children: kids, sizes, .. } if *d == dir => {
                    let at = kids.iter().position(|k| k.id() == tile).unwrap();
                    assert_eq!(at == 0, first, "{side:?}");
                    assert!((sizes[at] - 0.4).abs() < 1e-9, "{side:?} {sizes:?}");
                }
                // Beside b in the row: one of the row's children, just
                // before or after b (a's and b's sizes made room for it).
                _ => {
                    assert_eq!(dir, Dir::Row, "{side:?}");
                    let at = children.iter().position(|k| k.id() == tile).unwrap();
                    assert_eq!(at, if first { 1 } else { 2 }, "{side:?}");
                    assert_eq!(children.len(), 3);
                }
            }
            // The keyboard's tile stays focused.
            assert_ne!(s.space("s").unwrap().focused_pane_id.as_deref(), Some(tile.as_str()));
        }
    }

    #[test]
    fn a_squeezed_row_is_shared_evenly() {
        let s = state(tree::build_grid(&[3], &ids(&["a", "b", "c"])));
        let b = pane_of(&s, "b");
        let fit = Fit { area: (1700.0, 900.0), min: tree::MinSize { min_w: 380.0, min_h: 200.0 } };
        // Right of b at 0.45 of its third: b 312 wide, it folds. Even: four
        // of 425, nothing folds.
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Tile { pane: b, side: Side::Right }, 0.45, Some(fit));
        let layout = &s.space("s").unwrap().layout;
        let Node::Split { children, sizes, .. } = layout else { panic!() };
        assert_eq!(children.len(), 4);
        assert!(children.iter().any(|c| Some(c.id()) == tile.as_deref()));
        assert!(sizes.iter().all(|s| (s - 0.25).abs() < 1e-9), "{sizes:?}");
        assert!(folded(layout, Some(fit), None).is_empty());
    }

    #[test]
    fn edges_of_the_main_area_give_a_column_or_row_of_its_own() {
        let s = state(build_preset(Preset::G2x2, &ids(&["a", "b", "c", "d"])));
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Edge(Side::Right), 0.3, None);
        let layout = s.space("s").unwrap().layout.clone();
        let Node::Split { dir, children, sizes, .. } = &layout else { panic!() };
        assert_eq!(*dir, Dir::Row);
        assert_eq!(children.last().unwrap().id(), tile.unwrap());
        assert!((sizes.last().unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(layout.agents().len(), 4);
        // Moved to the top edge: one tile still, a row of its own.
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Edge(Side::Top), 0.3, None);
        let layout = s.space("s").unwrap().layout.clone();
        assert_eq!(layout.files_owners(), ids(&["a"]));
        let Node::Split { dir, children, .. } = &layout else { panic!() };
        assert_eq!((*dir, children[0].id()), (Dir::Col, tile.unwrap().as_str()));
    }

    #[test]
    fn a_tile_of_its_own_takes_an_empty_tile_first() {
        let s = state(build_preset(Preset::G2x2, &ids(&["a", "b"])));
        let empty = s.space("s").unwrap().layout.leaves().into_iter().find(|p| p.is_empty()).unwrap().id;
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Auto, 0.4, None);
        assert_eq!(tile.as_deref(), Some(empty.as_str()));
        assert_eq!(s.space("s").unwrap().layout.leaves().len(), 4);
        // A files tile isn't empty: an agent placed in the space goes to
        // the other empty tile, not over the files.
        let s = s.place_agent("s", "c", None, None);
        let layout = &s.space("s").unwrap().layout;
        assert_eq!(layout.find_files("a").unwrap().id, empty);
        assert!(layout.find_by_agent("c").is_some());
        // No empty tile left: the roomiest is split.
        let (s, tile) = s.dock_files("s", "z", &FilesSpot::Auto, 0.4, None);
        assert_eq!(s.space("s").unwrap().layout.leaves().len(), 5);
        assert!(tile.is_some());
    }

    #[test]
    fn one_files_tile_per_agent_moved_between_spaces_and_taken_out() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let a = pane_of(&s, "a");
        let (s, _) = s.dock_files("s", "a", &FilesSpot::Tile { pane: a.clone(), side: Side::Right }, 0.45, None);
        let only = s.space("t").unwrap().layout.leaves()[0].id.clone();
        let (s, tile) = s.dock_files("t", "a", &FilesSpot::Take(only.clone()), 0.45, None);
        assert_eq!(tile.as_deref(), Some(only.as_str()));
        assert!(s.space("s").unwrap().layout.find_files("a").is_none());
        assert_eq!(s.files_tile("a"), Some(("t".to_string(), only.clone())));
        // On its own tile: nothing happens.
        let (s2, none) = s.clone().dock_files("t", "a", &FilesSpot::Take(only.clone()), 0.45, None);
        assert_eq!((none, s2.space("t").unwrap().layout.clone()), (None, s.space("t").unwrap().layout.clone()));
        // Out: the space's only tile goes back to empty.
        let s = s.undock_files("a");
        assert!(s.files_tile("a").is_none());
        assert!(s.space("t").unwrap().layout.leaves()[0].is_empty());
    }

    #[test]
    fn presets_and_agent_drops_keep_files_tiles() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let b = pane_of(&s, "b");
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Tile { pane: b, side: Side::Bottom }, 0.45, None);
        let tile = tile.unwrap();
        // An agent dropped in the middle of the files tile: nothing.
        let (s, _, _) = s.drop_on_pane("s", "c", &tile, None, None);
        assert_eq!(s.space("s").unwrap().layout.find_files("a").unwrap().id, tile);
        assert!(s.space("s").unwrap().layout.find_by_agent("c").is_none());
        // On its side: a split, the files stay.
        let (s, _, _) = s.drop_on_pane("s", "c", &tile, Some(Side::Left), None);
        assert!(s.space("s").unwrap().layout.find_files("a").is_some());
        assert!(s.space("s").unwrap().layout.find_by_agent("c").is_some());
        // Re-tiled with a preset: still there, a column of its own.
        let s = s.apply_preset("s", Preset::Two, &[], |n| vec![n]);
        assert_eq!(s.space("s").unwrap().layout.files_owners(), ids(&["a"]));
        // It round-trips through ui.json.
        let json = serde_json::to_value(&s).unwrap();
        let back: UiState = serde_json::from_value(json).unwrap();
        assert_eq!(back.space("s").unwrap().layout, s.space("s").unwrap().layout);
    }

    #[test]
    fn what_a_files_tile_has_open_is_kept() {
        let s = UiState::default().keep_files("a", DockedFiles { key: "a".into(), tabs: ids(&["src/x.rs"]), active: Some("src/x.rs".into()) });
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["dockedFiles"]["a"]["tabs"][0], "src/x.rs");
        let back = UiState::sanitize(json);
        assert_eq!(back.docked_files["a"].active.as_deref(), Some("src/x.rs"));
        assert!(back.forget_files("a").docked_files.is_empty());
    }

    #[test]
    fn a_files_window_is_kept_until_it_closes_or_its_agent_goes() {
        let kept = FilesWindow {
            open: DockedFiles { key: "a".into(), tabs: ids(&["src/x.rs"]), active: Some("src/x.rs".into()) },
            expanded: ids(&["src"]),
            bounds: Some([10., 20., 900., 680.]),
        };
        let s = UiState::default().keep_files_window("a", kept.clone()).keep_files_window("b", FilesWindow::default());
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["filesWindows"]["a"]["tabs"][0], "src/x.rs");
        assert_eq!(json["filesWindows"]["a"]["expanded"][0], "src");
        let back = UiState::sanitize(json);
        assert_eq!(back.files_windows["a"], kept);
        // Its agent removed: forgotten; closed: forgotten.
        let pruned = back.clone().prune_agents(&ids(&["a"]));
        assert_eq!(pruned.files_windows.keys().cloned().collect::<Vec<_>>(), ids(&["a"]));
        assert!(pruned.forget_files_window("a").files_windows.is_empty());
    }

    #[test]
    fn the_old_split_inside_a_terminal_tile_leaves_nothing_behind() {
        // Before files tiles, a viewer could split an agent's terminal tile;
        // only its share was kept (`filesBeside`). It is read past and not
        // written again: files go in a layout slot of their own.
        let mut json = serde_json::to_value(UiState::default()).unwrap();
        json["filesBeside"] = serde_json::json!(0.4);
        let back = UiState::sanitize(json);
        assert!(serde_json::to_value(&back).unwrap().get("filesBeside").is_none());
    }

    /// `layout` in space "s" with "f"'s files dropped at `spot`: the
    /// layout and the files tile.
    fn dropped(layout: Node, spot: impl Fn(&UiState) -> FilesSpot) -> (Node, String) {
        let s = state(layout);
        let spot = spot(&s);
        let (s, tile) = s.dock_files("s", "f", &spot, FILES_SHARE, None);
        (s.space("s").unwrap().layout.clone(), tile.unwrap())
    }

    fn said(w: &Whereabouts) -> String {
        w.words(&|id: &str| id.to_string())
    }

    #[test]
    fn a_tile_is_said_next_to_the_terminal_it_was_placed_against() {
        use Whereabouts::*;
        for side in [Side::Right, Side::Left, Side::Top, Side::Bottom] {
            let (l, tile) = dropped(build_preset(Preset::Two, &ids(&["a", "b"])), |s| FilesSpot::Tile { pane: pane_of(s, "b"), side });
            assert_eq!(whereabouts(&l, &tile, Some("b")), Next(side, "b".into()), "{side:?}");
        }
        // From the tiles alone (after a restart: no anchor kept).
        let two = || build_preset(Preset::Two, &ids(&["a", "b"]));
        let (l, t) = dropped(two(), |s| FilesSpot::Tile { pane: pane_of(s, "b"), side: Side::Right });
        assert_eq!(said(&whereabouts(&l, &t, None)), "right of b's terminal");
        let (l, t) = dropped(two(), |s| FilesSpot::Tile { pane: pane_of(s, "b"), side: Side::Top });
        assert_eq!(said(&whereabouts(&l, &t, None)), "above b's terminal");
        let (l, t) = dropped(two(), |s| FilesSpot::Tile { pane: pane_of(s, "a"), side: Side::Bottom });
        assert_eq!(said(&whereabouts(&l, &t, None)), "below a's terminal");
        // An anchor it no longer touches: said from the tiles.
        let (l, t) = dropped(two(), |s| FilesSpot::Tile { pane: pane_of(s, "b"), side: Side::Right });
        assert_eq!(whereabouts(&l, &t, Some("a")), Next(Side::Right, "b".into()));
    }

    /// The case that was said wrong: dropped above demo (under docs), it
    /// was "below docs's terminal" from the order in the tree.
    #[test]
    fn dropped_above_demo_is_above_demo() {
        let docs = Node::pane(Some("docs".into()));
        let col = tree::split_with(&docs, docs.id(), Side::Bottom, &Node::pane(Some("demo".into())), 0.5);
        let (l, t) = dropped(col, |s| FilesSpot::Tile { pane: pane_of(s, "demo"), side: Side::Top });
        assert_eq!(said(&whereabouts(&l, &t, Some("demo"))), "above demo's terminal");
        // Without the anchor: between the two, each having a whole side.
        assert_eq!(said(&whereabouts(&l, &t, None)), "between docs's and demo's terminals");
    }

    #[test]
    fn between_two_terminals_nested_splits_and_edges() {
        use Whereabouts::*;
        // [a | files | b]: between them.
        let (l, t) = dropped(build_preset(Preset::Two, &ids(&["a", "b"])), |s| FilesSpot::Tile { pane: pane_of(s, "b"), side: Side::Left });
        assert_eq!(whereabouts(&l, &t, None), Between("a".into(), "b".into()));
        assert_eq!(said(&whereabouts(&l, &t, None)), "between a's and b's terminals");
        // A 2×2 grid, the files right of d (bottom right, in a nested row):
        // right of d, not of b above it.
        let grid = || build_grid(&[2, 2], &ids(&["a", "b", "c", "d"]));
        let (l, t) = dropped(grid(), |s| FilesSpot::Tile { pane: pane_of(s, "d"), side: Side::Right });
        assert_eq!(whereabouts(&l, &t, None), Next(Side::Right, "d".into()));
        let (l, t) = dropped(grid(), |s| FilesSpot::Tile { pane: pane_of(s, "a"), side: Side::Bottom });
        assert_eq!(whereabouts(&l, &t, None), Between("a".into(), "c".into()));
        // A column along the right edge, beside two terminals: an edge
        // column; a row along the bottom likewise.
        let (l, t) = dropped(grid(), |_| FilesSpot::Edge(Side::Right));
        assert_eq!(said(&whereabouts(&l, &t, None)), "a column on the right of the space");
        let (l, t) = dropped(grid(), |_| FilesSpot::Edge(Side::Bottom));
        assert_eq!(said(&whereabouts(&l, &t, None)), "a row along the bottom of the space");
        let (l, t) = dropped(grid(), |_| FilesSpot::Edge(Side::Left));
        assert_eq!(whereabouts(&l, &t, None), Edge(Side::Left));
        // Along the edge but beside one terminal only: that terminal's.
        let (l, t) = dropped(build_preset(Preset::Two, &ids(&["a", "b"])), |_| FilesSpot::Edge(Side::Right));
        assert_eq!(whereabouts(&l, &t, None), Next(Side::Right, "b".into()));
        // Nothing beside it but an empty tile.
        let (l, t) = dropped(Node::pane(None), |s| FilesSpot::Take(s.space("s").unwrap().layout.id().to_string()));
        assert_eq!(whereabouts(&l, &t, None), Alone);
    }

    /// After a restart, a files tile whose agent no longer exists isn't
    /// drawn again, and nothing of it stays in `ui.json`.
    #[test]
    fn files_of_a_removed_agent_leave_nothing_behind() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let b = pane_of(&s, "b");
        let (s, _) = s.dock_files("s", "gone", &FilesSpot::Tile { pane: b, side: Side::Right }, FILES_SHARE, None);
        let (s, _) = s.dock_files("s", "a", &FilesSpot::Edge(Side::Bottom), FILES_SHARE, None);
        let s = s.keep_files("gone", DockedFiles { key: "gone".into(), tabs: vec!["x.rs".into()], active: None });
        let s = s.keep_files("a", DockedFiles { key: "a".into(), tabs: vec![], active: None });
        let back: UiState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let next = back.clone().prune_agents(&ids(&["a", "b"]));
        let layout = &next.space("s").unwrap().layout;
        assert!(layout.find_files("gone").is_none() && !next.docked_files.contains_key("gone"));
        assert!(layout.find_files("a").is_some() && next.docked_files.contains_key("a"), "a's stay");
        assert_eq!(layout.agents(), ids(&["a", "b"]), "the gap closed, the terminals kept");
        assert!(!serde_json::to_string(&next).unwrap().contains("gone"));
        // One kept for unsaved edits (passed as alive) stays.
        let kept = back.prune_agents(&ids(&["a", "b", "gone"]));
        assert!(kept.space("s").unwrap().layout.find_files("gone").is_some());
    }

    /// Every split has a size for each child, summing to 1, and at least
    /// two children; no tile id twice; each terminal and files tile once.
    fn assert_valid(layout: &Node) {
        fn rec(n: &Node, seen: &mut Vec<String>) {
            match n {
                Node::Pane { id, .. } => {
                    assert!(!seen.contains(id), "tile {id} twice");
                    seen.push(id.clone());
                }
                Node::Split { children, sizes, .. } => {
                    assert!(children.len() >= 2, "a split of {} children", children.len());
                    assert_eq!(children.len(), sizes.len());
                    assert!((sizes.iter().sum::<f64>() - 1.0).abs() < 1e-6, "{sizes:?}");
                    for c in children {
                        rec(c, seen);
                    }
                }
            }
        }
        if let Node::Split { .. } = layout {
            rec(layout, &mut Vec::new());
        }
        let mut agents = layout.agents();
        let n = agents.len();
        agents.sort();
        agents.dedup();
        assert_eq!(agents.len(), n, "a terminal twice");
        let mut owners = layout.files_owners();
        let n = owners.len();
        owners.dedup();
        assert_eq!(owners.len(), n, "files twice");
    }

    /// Each tile's place and size on a 1×1 area.
    fn rects(layout: &Node) -> Vec<(String, (f64, f64, f64, f64))> {
        tree::pane_rects(layout, tree::Rect { x: 0., y: 0., w: 1., h: 1. }).into_iter().map(|(id, r)| (id, (r.x, r.y, r.w, r.h))).collect()
    }

    #[test]
    fn files_replace_a_terminal_in_its_place() {
        let s = state(build_preset(Preset::G2x2, &ids(&["a", "b", "c", "d"])));
        let b = pane_of(&s, "b");
        let before = rects(&s.space("s").unwrap().layout);
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Replace(b.clone()), 0.45, None);
        let sp = s.space("s").unwrap();
        // The same tile, the same place and size: b's terminal is out.
        assert_eq!(tile.as_deref(), Some(b.as_str()));
        assert_eq!(sp.layout.find_files("a").unwrap().id, b);
        assert_eq!(rects(&sp.layout), before);
        assert!(sp.layout.find_by_agent("b").is_none());
        assert_eq!(sp.layout.agents().len(), 3);
        // b stays a member of the space (as a closed tile's agent does).
        assert!(sp.members.contains(&"b".to_string()));
        assert_valid(&sp.layout);
        // Its own terminal: the same.
        let s2 = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let a = pane_of(&s2, "a");
        let (s2, tile) = s2.dock_files("s", "a", &FilesSpot::Replace(a.clone()), 0.45, None);
        assert_eq!(tile.as_deref(), Some(a.as_str()));
        assert_eq!(s2.space("s").unwrap().layout.agents(), ids(&["b"]));
        // An empty tile: taken, as before.
        let s3 = state(build_preset(Preset::G2x2, &ids(&["a"])));
        let empty = s3.space("s").unwrap().layout.leaves().into_iter().find(|p| p.is_empty()).unwrap().id;
        let (s3, tile) = s3.dock_files("s", "a", &FilesSpot::Replace(empty.clone()), 0.45, None);
        assert_eq!(tile, Some(empty));
        assert_valid(&s3.space("s").unwrap().layout);
    }

    #[test]
    fn a_replaced_terminal_comes_back_and_the_layout_stays_whole() {
        let s = state(build_preset(Preset::G2x2, &ids(&["a", "b", "c", "d"])));
        let b = pane_of(&s, "b");
        // Files right of c first, then onto b's terminal (moved there).
        let c = pane_of(&s, "c");
        let (s, _) = s.dock_files("s", "a", &FilesSpot::Tile { pane: c, side: Side::Right }, 0.45, None);
        assert_valid(&s.space("s").unwrap().layout);
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Replace(b.clone()), 0.45, None);
        assert_eq!(tile.as_deref(), Some(b.as_str()));
        let layout = s.space("s").unwrap().layout.clone();
        assert_valid(&layout);
        assert_eq!(layout.files_owners(), ids(&["a"]));
        assert_eq!(layout.leaves().len(), 4);
        // b shown again (the sidebar): beside the files, never over them.
        let focused = s.space("s").unwrap().focused_pane_id.clone();
        let s = s.focus_pane("s", &b).place_agent("s", "b", None, None);
        let layout = s.space("s").unwrap().layout.clone();
        assert_valid(&layout);
        assert!(layout.find_by_agent("b").is_some());
        assert_eq!(layout.find_files("a").unwrap().id, b);
        let _ = focused;
        // The files moved along an edge, then out to a window.
        let (s, _) = s.dock_files("s", "a", &FilesSpot::Edge(Side::Bottom), 0.45, None);
        assert_valid(&s.space("s").unwrap().layout);
        let s = s.undock_files("a");
        let layout = s.space("s").unwrap().layout.clone();
        assert_valid(&layout);
        assert!(layout.files_owners().is_empty());
        let mut agents = layout.agents();
        agents.sort();
        assert_eq!(agents, ids(&["a", "b", "c", "d"]));
        assert!(layout.leaves().iter().all(|p| p.agent_id.is_some() || p.is_empty()));
    }

    #[test]
    fn files_dropped_on_other_files_swap_places_in_one_space() {
        let s = state(build_preset(Preset::Three, &ids(&["a", "b", "c"])));
        let (b, c) = (pane_of(&s, "b"), pane_of(&s, "c"));
        let (s, fa) = s.dock_files("s", "a", &FilesSpot::Tile { pane: b, side: Side::Right }, 0.45, None);
        let (s, fz) = s.dock_files("s", "z", &FilesSpot::Tile { pane: c, side: Side::Bottom }, 0.45, None);
        let (fa, fz) = (fa.unwrap(), fz.unwrap());
        let before = rects(&s.space("s").unwrap().layout);
        let (s, tile) = s.dock_files("s", "a", &FilesSpot::Replace(fz.clone()), 0.45, None);
        let layout = s.space("s").unwrap().layout.clone();
        assert_eq!(tile.as_deref(), Some(fz.as_str()));
        assert_eq!(layout.find_files("a").unwrap().id, fz);
        assert_eq!(layout.find_files("z").unwrap().id, fa);
        assert_eq!(rects(&layout), before, "the tiles stay as they were");
        assert_valid(&layout);
        // Files not in this space: nothing (the others would have nowhere to go).
        let (s2, tile) = s.clone().dock_files("s", "w", &FilesSpot::Replace(fz.clone()), 0.45, None);
        assert_eq!(tile, None);
        assert_eq!(s2.space("s").unwrap().layout, layout);
        // Its own tile: nothing.
        let (_, tile) = s.dock_files("s", "a", &FilesSpot::Replace(fz), 0.45, None);
        assert_eq!(tile, None);
    }
}
