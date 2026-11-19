//! Web tiles in a space's layout (docs/spec/web.md): a web page in a tile
//! of its own, a peer of the terminal and files tiles, placed, moved and
//! swapped by the same code as files tiles ([`UiState::dock`] with
//! [`Content::Web`]). The pane holds the tile's id; `webTiles` in
//! `ui.json` holds its page, so the tile comes back on it after a restart.
//!
//! Two kinds, one tile: a page opened by the user ("Open web page…",
//! owned by no agent) and an agent's Preview (`agent`: the dev server its
//! terminal printed; one per agent, it goes with the agent). Pure, as the
//! rest of [`super`].

use serde::{Deserialize, Serialize};

use super::{FilesSpot, Fit, UiState};
use crate::main_screen::tree::{self, Content};

/// A web tile's share of what it splits, at first.
pub const WEB_SHARE: f64 = 0.5;

/// A web tile's page, kept in `ui.json` (`webTiles`, by tile id).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebTile {
    /// The page's address as last shown (empty: none typed yet).
    #[serde(default)]
    pub url: String,
    /// An agent's Preview: that agent's id. It follows what its terminal
    /// prints and closes with the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// A fresh web tile id.
pub fn new_id() -> String {
    tree::new_id("web")
}

impl UiState {
    /// The space and pane showing web tile `id`.
    pub fn web_tile(&self, id: &str) -> Option<(String, String)> {
        self.tile_of(&Content::Web(id.to_string()))
    }

    /// The web tile that is agent `agent`'s Preview.
    pub fn preview_of(&self, agent: &str) -> Option<String> {
        self.web_tiles
            .iter()
            .find(|(_, t)| t.agent.as_deref() == Some(agent))
            .map(|(id, _)| id.clone())
    }

    /// Web tile `id` showing `tile`, put at `spot` of space `space_id` (or
    /// moved there). Returns the pane, or `None` when the spot isn't in
    /// that space (nothing changed). An agent's Preview replaces any it had.
    pub fn open_web(self, space_id: &str, id: &str, tile: WebTile, spot: &FilesSpot, fit: Option<Fit>) -> (UiState, Option<String>) {
        let mut next = self;
        if let Some(old) = tile.agent.as_deref().and_then(|a| next.preview_of(a)).filter(|old| old != id) {
            next = next.close_web(&old);
        }
        let (mut next, placed) = next.dock(space_id, &Content::Web(id.to_string()), spot, WEB_SHARE, fit);
        if placed.is_some() {
            next.web_tiles.insert(id.to_string(), tile);
        }
        (next, placed)
    }

    /// Web tile `id` closed: out of the layout, its page forgotten.
    pub fn close_web(self, id: &str) -> UiState {
        let mut next = self.undock(&Content::Web(id.to_string()));
        next.web_tiles.remove(id);
        next
    }

    /// The page web tile `id` shows now (kept for the next start).
    pub fn set_web_url(mut self, id: &str, url: &str) -> UiState {
        if let Some(t) = self.web_tiles.get_mut(id) {
            t.url = url.to_string();
        }
        self
    }

    /// Previews of agents not in `alive` close; a page with no tile, and a
    /// tile with no page, go.
    pub fn prune_web(self, alive: &[String]) -> UiState {
        let in_layout: Vec<String> = self.spaces.iter().flat_map(|s| s.layout.web_ids()).collect();
        let gone: Vec<String> = in_layout
            .iter()
            .filter(|id| match self.web_tiles.get(*id) {
                None => true,
                Some(t) => t.agent.as_ref().is_some_and(|a| !alive.contains(a)),
            })
            .cloned()
            .collect();
        let mut next = self;
        for id in gone {
            next = next.close_web(&id);
        }
        next.web_tiles.retain(|id, _| in_layout.contains(id));
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_screen::tree::{build_preset, Node, Preset, Side};
    use crate::main_screen::workspace::SpaceKind;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

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
        s
    }

    fn pane_of(s: &UiState, agent: &str) -> String {
        s.space("s").unwrap().layout.find_by_agent(agent).unwrap().id
    }

    fn page(url: &str) -> WebTile {
        WebTile { url: url.into(), agent: None }
    }

    #[test]
    fn a_web_tile_opens_beside_a_terminal_and_moves_like_files() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let b = pane_of(&s, "b");
        let (s, pane) = s.open_web("s", "w1", page("https://example.com/"), &FilesSpot::Tile { pane: b.clone(), side: Side::Right }, None);
        let pane = pane.expect("placed");
        let layout = s.space("s").unwrap().layout.clone();
        assert_eq!(layout.web_ids(), ids(&["w1"]));
        assert_eq!(layout.find_content(&Content::Web("w1".into())).unwrap().id, pane);
        assert_eq!(layout.agents(), ids(&["a", "b"]), "the terminals stay");
        assert_eq!(s.web_tile("w1"), Some(("s".to_string(), pane.clone())));
        // The keyboard's tile stays focused (the page's field takes the keys).
        assert_ne!(s.space("s").unwrap().focused_pane_id.as_deref(), Some(pane.as_str()));
        // Moved along the bottom edge, then into the other space: one tile.
        let (s, _) = s.dock("s", &Content::Web("w1".into()), &FilesSpot::Edge(Side::Bottom), WEB_SHARE, None);
        assert_eq!(s.space("s").unwrap().layout.web_ids(), ids(&["w1"]));
        let only = s.space("t").unwrap().layout.leaves()[0].id.clone();
        let (s, at) = s.dock("t", &Content::Web("w1".into()), &FilesSpot::Take(only.clone()), WEB_SHARE, None);
        assert_eq!(at.as_deref(), Some(only.as_str()));
        assert!(s.space("s").unwrap().layout.web_ids().is_empty());
        assert_eq!(s.web_tiles["w1"].url, "https://example.com/");
        // Closed: out of the layout and forgotten.
        let s = s.close_web("w1");
        assert!(s.web_tile("w1").is_none() && s.web_tiles.is_empty());
        assert!(s.space("t").unwrap().layout.leaves()[0].is_empty());
    }

    #[test]
    fn a_web_tile_replaces_a_terminal_and_swaps_with_files() {
        let s = state(build_preset(Preset::Three, &ids(&["a", "b", "c"])));
        let (a, b) = (pane_of(&s, "a"), pane_of(&s, "b"));
        // Onto b's terminal: its place, b leaves the layout (keeps running).
        let (s, pane) = s.open_web("s", "w", page("http://localhost:3000/"), &FilesSpot::Replace(b.clone()), None);
        assert_eq!(pane.as_deref(), Some(b.as_str()));
        let layout = s.space("s").unwrap().layout.clone();
        assert!(layout.find_by_agent("b").is_none());
        assert!(s.space("s").unwrap().members.contains(&"b".to_string()));
        // Files beside a, then the web tile dropped onto the files: swapped.
        let (s, files) = s.dock_files("s", "a", &FilesSpot::Tile { pane: a, side: Side::Bottom }, 0.4, None);
        let files = files.unwrap();
        let (s, at) = s.dock("s", &Content::Web("w".into()), &FilesSpot::Replace(files.clone()), WEB_SHARE, None);
        assert_eq!(at.as_deref(), Some(files.as_str()));
        let layout = s.space("s").unwrap().layout.clone();
        assert_eq!(layout.find_content(&Content::Web("w".into())).unwrap().id, files);
        assert_eq!(layout.find_files("a").unwrap().id, b);
        // An agent dropped in its middle: nothing; on its side: a split.
        let (s2, _, _) = s.clone().drop_on_pane("s", "b", &files, None, None);
        assert_eq!(s2.space("s").unwrap().layout, layout);
        let (s2, _, _) = s.drop_on_pane("s", "b", &files, Some(Side::Left), None);
        assert!(s2.space("s").unwrap().layout.find_by_agent("b").is_some());
        assert!(s2.space("s").unwrap().layout.find_content(&Content::Web("w".into())).is_some());
    }

    #[test]
    fn presets_and_agents_never_take_a_web_tile() {
        let s = state(build_preset(Preset::G2x2, &ids(&["a"])));
        let (s, _) = s.open_web("s", "w", page("https://example.com/"), &FilesSpot::Auto, None);
        // An empty tile was taken; an agent placed goes elsewhere.
        let s = s.place_agent("s", "b", None, None);
        let layout = s.space("s").unwrap().layout.clone();
        assert!(layout.find_by_agent("b").is_some());
        assert_eq!(layout.web_ids(), ids(&["w"]));
        // Re-tiled: still there, a column on the right.
        let s = s.apply_preset("s", Preset::Two, &[], |n| vec![n]);
        assert_eq!(s.space("s").unwrap().layout.web_ids(), ids(&["w"]));
    }

    #[test]
    fn a_preview_is_one_per_agent_and_goes_with_it() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let a = pane_of(&s, "a");
        let tile = WebTile { url: "http://localhost:5173/".into(), agent: Some("a".into()) };
        let (s, _) = s.open_web("s", "p1", tile.clone(), &FilesSpot::Tile { pane: a.clone(), side: Side::Right }, None);
        assert_eq!(s.preview_of("a").as_deref(), Some("p1"));
        // Another Preview for a: the first one goes.
        let (s, _) = s.open_web("s", "p2", tile, &FilesSpot::Tile { pane: a, side: Side::Right }, None);
        assert_eq!(s.preview_of("a").as_deref(), Some("p2"));
        assert_eq!(s.space("s").unwrap().layout.web_ids(), ids(&["p2"]));
        // A free page stays when agents go; a's Preview goes with a.
        let (s, _) = s.open_web("s", "w", page("https://example.com/"), &FilesSpot::Edge(Side::Bottom), None);
        let s = s.prune_agents(&ids(&["b"]));
        assert!(s.preview_of("a").is_none());
        assert_eq!(s.space("s").unwrap().layout.web_ids(), ids(&["w"]));
        assert_eq!(s.web_tiles.keys().cloned().collect::<Vec<_>>(), ids(&["w"]));
    }

    #[test]
    fn web_tiles_come_back_after_a_restart() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let b = pane_of(&s, "b");
        let (s, _) = s.open_web("s", "w", page("https://example.com/"), &FilesSpot::Tile { pane: b, side: Side::Left }, None);
        let s = s.set_web_url("w", "https://example.com/docs");
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["webTiles"]["w"]["url"], "https://example.com/docs");
        assert!(json["webTiles"]["w"].get("agent").is_none());
        let back = UiState::sanitize(json);
        assert_eq!(back.space("s").unwrap().layout, s.space("s").unwrap().layout);
        assert_eq!(back.web_tiles["w"].url, "https://example.com/docs");
        // Pruned with every agent alive: nothing changes.
        assert_eq!(back.clone().prune_agents(&ids(&["a", "b"])), back);
    }

    #[test]
    fn an_old_ui_json_loads_and_drops_the_in_tile_preview_flags() {
        // Before web tiles: no `web` on panes, no `webTiles`, and where a
        // Preview sat inside its terminal's tile.
        let raw = serde_json::json!({
            "v": 1,
            "spaces": [{
                "id": "all", "name": "All", "kind": "all", "members": [],
                "layout": {"type": "split", "id": "s1", "dir": "row", "sizes": [0.5, 0.5], "children": [
                    {"type": "pane", "id": "p1", "agentId": "a"},
                    {"type": "pane", "id": "p2", "agentId": null, "files": "a"}
                ]},
                "focusedPaneId": "p1"
            }],
            "previewBelow": true,
            "previewLeft": true
        });
        let s = UiState::sanitize(raw);
        let layout = &s.space("all").unwrap().layout;
        assert_eq!(layout.agents(), ids(&["a"]));
        assert_eq!(layout.files_owners(), ids(&["a"]));
        assert!(layout.web_ids().is_empty() && s.web_tiles.is_empty());
        let out = serde_json::to_value(&s).unwrap();
        assert!(out.get("previewBelow").is_none() && out.get("previewLeft").is_none());
        assert!(out.get("webTiles").is_none());
        assert!(out["spaces"][0]["layout"]["children"][0].get("web").is_none());
    }

    #[test]
    fn a_tile_without_its_page_and_a_page_without_its_tile_go() {
        let s = state(build_preset(Preset::Two, &ids(&["a", "b"])));
        let (mut s, _) = s.open_web("s", "w", page("https://example.com/"), &FilesSpot::Auto, None);
        s.web_tiles.clear();
        s.web_tiles.insert("orphan".into(), page("https://example.org/"));
        let s = s.prune_web(&ids(&["a", "b"]));
        assert!(s.space("s").unwrap().layout.web_ids().is_empty());
        assert!(s.web_tiles.is_empty());
        assert_eq!(s.space("s").unwrap().layout.agents(), ids(&["a", "b"]));
    }
}
