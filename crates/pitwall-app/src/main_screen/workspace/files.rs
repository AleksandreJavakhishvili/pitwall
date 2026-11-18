//! Files tiles in a space's layout: an agent's file viewer placed among
//! the terminal tiles, wherever the user (or the agent, asked by the user)
//! puts it. One files tile per agent across every space: placing it again
//! moves it. Pure, as the rest of [`super`].

use serde::{Deserialize, Serialize};

use super::{auto_spot, empty_layout, folded, Fit, UiState};
use crate::main_screen::tree::{self, Node, Side};

/// Where a files tile goes.
#[derive(Debug, Clone, PartialEq)]
pub enum FilesSpot {
    /// On `side` of tile `pane` (splitting it).
    Tile { pane: String, side: Side },
    /// Along `side` of the whole main area: a column or row of its own.
    Edge(Side),
    /// In empty tile `pane`.
    Take(String),
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

/// A files tile's share of what it splits, at first.
pub const FILES_SHARE: f64 = 0.45;
/// Along an edge of the main area.
pub const EDGE_SHARE: f64 = 0.35;

impl UiState {
    /// The space and tile showing agent `owner`'s files.
    pub fn files_tile(&self, owner: &str) -> Option<(String, String)> {
        self.spaces
            .iter()
            .find_map(|sp| sp.layout.find_files(owner).map(|p| (sp.id.clone(), p.id)))
    }

    /// Agent `owner`'s files tile taken out of every space (what it had
    /// open is kept until [`UiState::forget_files`]).
    pub fn undock_files(mut self, owner: &str) -> UiState {
        for sp in &mut self.spaces {
            let Some(p) = sp.layout.find_files(owner) else { continue };
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
        let Some(sp) = self.space(space_id) else { return (self, None) };
        let own = sp.layout.find_files(owner).map(|p| p.id);
        let target_ok = match spot {
            FilesSpot::Tile { pane, .. } | FilesSpot::Take(pane) => {
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
        let next = self.undock_files(owner);
        let fresh = Node::files_pane(owner);
        let id = fresh.id().to_string();
        let mut placed = Some(id.clone());
        let next = next.map_space(space_id, |mut sp| {
            let only_empty = sp.layout.leaves().len() == 1 && sp.layout.leaves()[0].is_empty();
            let before = sp.layout.clone();
            sp.layout = match spot {
                FilesSpot::Tile { pane, side } => tree::split_with(&sp.layout, pane, *side, &fresh, share),
                FilesSpot::Edge(_) if only_empty => fresh.clone(),
                FilesSpot::Edge(side) => tree::wrap_edge(&sp.layout, *side, &fresh, share.min(EDGE_SHARE)),
                FilesSpot::Take(pane) => {
                    placed = Some(pane.clone());
                    take(&sp.layout, pane, owner)
                }
                FilesSpot::Auto => match sp.layout.leaves().into_iter().find(|p| p.is_empty()) {
                    Some(empty) => {
                        placed = Some(empty.id.clone());
                        take(&sp.layout, &empty.id, owner)
                    }
                    None => {
                        let (at, side) = auto_spot(&sp.layout, fit);
                        tree::split_with(&sp.layout, &at, side, &fresh, 0.5)
                    }
                },
            };
            // Tiles it squeezed until they fold: its row (or column) shared
            // evenly instead, when that folds fewer.
            if let Some(tile) = placed.as_deref() {
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

/// `layout` with empty tile `pane` showing `owner`'s files.
fn take(layout: &Node, pane: &str, owner: &str) -> Node {
    fn rec(n: &Node, pane: &str, owner: &str) -> Node {
        match n {
            Node::Pane { id, .. } if id == pane => Node::Pane { id: id.clone(), agent_id: None, files: Some(owner.to_string()) },
            Node::Pane { .. } => n.clone(),
            Node::Split { id, dir, children, sizes } => Node::Split {
                id: id.clone(),
                dir: *dir,
                children: children.iter().map(|c| rec(c, pane, owner)).collect(),
                sizes: sizes.clone(),
            },
        }
    }
    rec(layout, pane, owner)
}

/// `new` (a layout rebuilt from its agents: a preset) with the files tiles
/// of `old` it lost, each a column of its own on the right.
pub fn keep_files_tiles(old: &Node, new: Node) -> Node {
    let mut out = new;
    for owner in old.files_owners() {
        if out.find_files(&owner).is_none() {
            out = tree::wrap_edge(&out, Side::Right, &Node::files_pane(&owner), EDGE_SHARE);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_screen::tree::{build_preset, Dir, Preset};
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
    fn the_old_split_inside_a_terminal_tile_leaves_nothing_behind() {
        // Before files tiles, a viewer could split an agent's terminal tile;
        // only its share was kept (`filesBeside`). It is read past and not
        // written again: files go in a layout slot of their own.
        let mut json = serde_json::to_value(UiState::default()).unwrap();
        json["filesBeside"] = serde_json::json!(0.4);
        let back = UiState::sanitize(json);
        assert!(serde_json::to_value(&back).unwrap().get("filesBeside").is_none());
    }
}
