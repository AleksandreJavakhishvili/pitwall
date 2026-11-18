//! The viewer's file tree column (Explorer / Search on the left), a
//! [`crate::kit::side`] column: shown or hidden (the sidebar button in the
//! viewer's bar, ⌘⇧E) and as wide as it was dragged. Both are kept in
//! `ui.json` (`filesTree`) for each kind of place a viewer is drawn in,
//! because a files tile in the layout wants another layout than a window
//! of its own or the full viewer. A viewer too narrow for the tree and a
//! readable file starts with the tree hidden, so the file the agent showed
//! gets the room; the user can still open it.

use serde::{Deserialize, Serialize};

use super::viewer::Host;
use crate::kit::side::{Limits, SideState};

/// One kind of place's column.
pub type Column = SideState;

/// The column is never narrower than this: names stay readable, and the
/// Explorer / Search switch and the eye beside it fit…
pub const MIN_W: f32 = 176.;
/// …nor wider than this.
pub const MAX_W: f32 = 520.;
/// The file keeps at least this much beside the tree. A viewer narrower
/// than the tree and this starts with the tree hidden.
pub const MIN_FILE_W: f32 = 380.;

/// `ui.json` `filesTree`: the column in the full viewer, in a files tile
/// and in a window. (A files tile's was `beside` when the files were a
/// pane split beside a terminal: read once, written as `tile`.)
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesTree {
    #[serde(default)]
    pub main: Column,
    #[serde(default, alias = "beside")]
    pub tile: Column,
    #[serde(default)]
    pub window: Column,
}

impl FilesTree {
    pub fn get(&self, host: Host) -> Column {
        match host {
            Host::Main => self.main,
            Host::Tile => self.tile,
            Host::Window => self.window,
        }
    }

    pub fn get_mut(&mut self, host: Host) -> &mut Column {
        match host {
            Host::Main => &mut self.main,
            Host::Tile => &mut self.tile,
            Host::Window => &mut self.window,
        }
    }

    /// Nothing chosen anywhere (not written to `ui.json`).
    pub fn is_empty(&self) -> bool {
        *self == FilesTree::default()
    }
}

/// The column's width before any drag: the full viewer has more room.
pub fn default_width(host: Host) -> f32 {
    match host {
        Host::Main => 280.,
        Host::Tile | Host::Window => 232.,
    }
}

/// The column's widths for `host`.
pub fn limits(host: Host) -> Limits {
    Limits { min: MIN_W, max: MAX_W, keep: MIN_FILE_W, default: default_width(host) }
}

/// `w` kept between the limits, and (in a viewer `viewer_w` wide) leaving
/// the file its room.
pub fn clamp_width(w: f32, viewer_w: Option<f32>) -> f32 {
    limits(Host::Window).clamp(w, viewer_w)
}

/// The column's width for `host` in a viewer `viewer_w` wide.
pub fn width(col: Column, host: Host, viewer_w: Option<f32>) -> f32 {
    limits(host).width(col, viewer_w)
}

/// The tree is shown: as toggled in this viewer (`session`), else as kept
/// for its kind of place, except that a narrow viewer starts without it.
pub fn shown(session: Option<bool>, col: Column, host: Host, viewer_w: Option<f32>) -> bool {
    limits(host).shown(session, col, viewer_w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_place_keeps_its_own_column() {
        let mut ft = FilesTree::default();
        assert!(ft.is_empty());
        ft.get_mut(Host::Tile).open = Some(false);
        ft.get_mut(Host::Window).width = Some(300.);
        assert_eq!(ft.get(Host::Tile).open, Some(false));
        assert_eq!(ft.get(Host::Main), Column::default());
        assert_eq!(ft.get(Host::Window).width, Some(300.));
        // In ui.json's words, and back.
        let json = serde_json::to_value(ft).unwrap();
        assert_eq!(json["tile"]["open"], false);
        // Kept by an older Pitwall as `beside`: read as the tile's.
        let old: FilesTree = serde_json::from_value(serde_json::json!({"beside": {"open": false}})).unwrap();
        assert_eq!(old.tile.open, Some(false));
        assert_eq!(json["window"]["width"], 300.);
        assert!(json["main"].as_object().unwrap().is_empty());
        let back: FilesTree = serde_json::from_value(json).unwrap();
        assert_eq!(back, ft);
        // Missing keys: nothing chosen.
        let back: FilesTree = serde_json::from_str("{}").unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn toggling_wins_over_what_was_kept_and_the_width() {
        let kept_open = Column { open: Some(true), width: None };
        let kept_closed = Column { open: Some(false), width: None };
        assert!(!shown(Some(false), kept_open, Host::Window, Some(1200.)));
        assert!(shown(Some(true), kept_closed, Host::Tile, Some(300.)));
        assert!(!shown(None, kept_closed, Host::Window, Some(1200.)));
        assert!(shown(None, kept_open, Host::Window, Some(1200.)));
        assert!(shown(None, Column::default(), Host::Main, Some(1200.)));
        assert!(shown(None, Column::default(), Host::Tile, None));
    }

    #[test]
    fn a_narrow_pane_starts_with_the_tree_hidden() {
        let col = Column::default();
        // A files tile at 45 % of a laptop window.
        assert!(!shown(None, col, Host::Tile, Some(420.)));
        // Kept open there before: still hidden at first, the file needs it.
        assert!(!shown(None, Column { open: Some(true), width: None }, Host::Tile, Some(420.)));
        assert!(shown(None, col, Host::Tile, Some(default_width(Host::Tile) + MIN_FILE_W + 1.)));
        // The full viewer's wider default needs more room.
        assert!(!shown(None, col, Host::Main, Some(640.)));
    }

    #[test]
    fn the_width_stays_between_its_limits_and_leaves_the_file_room() {
        assert_eq!(clamp_width(40., None), MIN_W);
        assert_eq!(clamp_width(2000., None), MAX_W);
        assert_eq!(clamp_width(500., Some(700.)), 700. - MIN_FILE_W);
        assert_eq!(width(Column::default(), Host::Main, None), 280.);
        assert_eq!(width(Column { open: None, width: Some(200.) }, Host::Window, Some(1000.)), 200.);
    }
}
