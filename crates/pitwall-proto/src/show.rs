//! `viewer.open`: an agent (or the user's own `pitwall open`) shows files
//! and folders to the user in Pitwall's viewer. The `pitwall` MCP server
//! Pitwall gives the agents it starts is a thin wrapper over this call.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The MCP server's name in the agents' config (`mcp__pitwall__…` in
/// Claude Code).
pub const MCP_SERVER: &str = "pitwall";
/// The MCP tool that opens files as tabs.
pub const TOOL_SHOW_FILES: &str = "show_files";
/// The MCP tool that opens a folder as a tree.
pub const TOOL_SHOW_FOLDER: &str = "show_folder";
/// Both tools.
pub const TOOLS: &[&str] = &[TOOL_SHOW_FILES, TOOL_SHOW_FOLDER];

/// Where the files show: a tile in the layout of the Pitwall window showing
/// the calling agent, on that side of its terminal (or of the terminal
/// `of` names), or a window of their own. Not given: right of the
/// terminal. `beside` is read as `right`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ViewerPlace {
    /// A tile right of the terminal.
    #[default]
    Right,
    Left,
    Above,
    Below,
    /// A window of its own.
    Window,
}

// By hand: the words [`ViewerPlace::parse`] takes (`beside` among them).
impl<'de> Deserialize<'de> for ViewerPlace {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ViewerPlace::parse(&s).ok_or_else(|| serde::de::Error::unknown_variant(&s, ViewerPlace::WORDS))
    }
}

impl ViewerPlace {
    /// Every word `where` takes, as the tools list them.
    pub const WORDS: &'static [&'static str] = &["right", "left", "above", "below", "window"];

    pub fn parse(s: &str) -> Option<ViewerPlace> {
        match s.trim().to_ascii_lowercase().as_str() {
            "right" | "beside" => Some(ViewerPlace::Right),
            "left" => Some(ViewerPlace::Left),
            "above" | "top" => Some(ViewerPlace::Above),
            "below" | "bottom" => Some(ViewerPlace::Below),
            "window" => Some(ViewerPlace::Window),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            ViewerPlace::Right => "right",
            ViewerPlace::Left => "left",
            ViewerPlace::Above => "above",
            ViewerPlace::Below => "below",
            ViewerPlace::Window => "window",
        }
    }

    /// A tile in the layout (not a window).
    pub fn is_tile(self) -> bool {
        self != ViewerPlace::Window
    }
}

/// `viewer.open`: files (each `path`, `path:line` or `path:line:col`) as
/// tabs, or one folder as a tree. Relative paths are taken from `root`,
/// else `cwd`, else the agent's folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ViewerOpen {
    /// Whose viewer (id or name). Default: the agent the caller runs in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// A folder or a whole worktree, shown as a tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// What relative paths are inside: a folder, or a worktree of the
    /// agent's repository by its folder name or branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The caller's folder (a shell's), for relative paths without `root`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Where (`where` on the wire). `None`: right of the terminal, or
    /// where the agent's files already are.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub place: Option<ViewerPlace>,
    /// Whose terminal `where` is next to (an agent's id or name, in the
    /// same space). Default: the caller's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
}

/// One file as it was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenedFile {
    /// Its full path.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// A path that wasn't opened, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

/// What `viewer.open` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ViewerOpened {
    /// The agent whose viewer shows it (the caller's, or the agent working
    /// in the folder asked for).
    pub agent_id: String,
    pub agent_name: String,
    /// The window it opened in (`main`, `pitwall-2`, …).
    pub window: String,
    /// Pitwall was not in front, so it was opened there without coming
    /// forward.
    pub in_background: bool,
    pub opened: Vec<OpenedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<Skipped>,
    /// Where it opened.
    pub place: ViewerPlace,
    /// Where it was asked to open (`None`: Pitwall picked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<ViewerPlace>,
    /// Where it is, in words ("a files tile in space Main (window main),
    /// above api's terminal"; "…, a column on the right of the space").
    pub shown_in: String,
    /// Why it opened somewhere else than asked, or how Pitwall picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What the server hands the app once paths are resolved: full paths on the
/// agent's machine, checked to exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ViewerShow {
    /// The agent that asked (its files tile goes next to its terminal).
    pub agent_id: String,
    pub files: Vec<OpenedFile>,
    pub folder: Option<String>,
    /// The `root` asked for, resolved (a worktree, a folder).
    #[serde(default)]
    pub root: Option<String>,
    /// `None`: Pitwall picks.
    pub place: Option<ViewerPlace>,
    /// The agent whose terminal `place` is next to (resolved id).
    #[serde(default)]
    pub of: Option<String>,
}

/// Where the app showed a [`ViewerShow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerShown {
    /// The agent the viewer belongs to (the one that asked).
    pub agent_id: String,
    /// The Pitwall window its tile is in (`main`, `pitwall-2`, …).
    pub window: String,
    pub in_background: bool,
    /// Where it opened.
    pub place: ViewerPlace,
    /// Where it is, in words (a tile: next to whose terminal, in which
    /// space and window).
    #[serde(default)]
    pub at: Option<String>,
    /// The folder its tree shows.
    pub root: String,
    /// Why it opened somewhere else than asked, or how Pitwall picked.
    #[serde(default)]
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn where_is_spelled_where_on_the_wire() {
        let r = ViewerOpen { files: vec!["a.rs:3".into()], place: Some(ViewerPlace::Window), ..Default::default() };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"files": ["a.rs:3"], "where": "window"}));
        assert_eq!(serde_json::from_value::<ViewerOpen>(v).unwrap(), r);
        assert_eq!(serde_json::from_value::<ViewerOpen>(json!({})).unwrap(), ViewerOpen::default());
        assert_eq!(ViewerPlace::parse(" Beside "), Some(ViewerPlace::Right));
        assert_eq!(ViewerPlace::parse("above"), Some(ViewerPlace::Above));
        assert_eq!(ViewerPlace::parse("tab"), None);
        // `beside` (the old word) still reads as right.
        assert_eq!(serde_json::from_value::<ViewerOpen>(json!({"where": "beside"})).unwrap().place, Some(ViewerPlace::Right));
        let r = ViewerOpen { place: Some(ViewerPlace::Below), of: Some("web".into()), ..Default::default() };
        assert_eq!(serde_json::to_value(&r).unwrap(), json!({"where": "below", "of": "web"}));
        for w in ViewerPlace::WORDS {
            assert_eq!(ViewerPlace::parse(w).map(ViewerPlace::word), Some(*w));
        }
    }
}
