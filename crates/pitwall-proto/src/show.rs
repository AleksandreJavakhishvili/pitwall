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

/// Where the user wants it. Carried through and kept; the viewer is the
/// one place both open in today (a pane beside the agent's terminal and a
/// window of its own come next).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ViewerPlace {
    /// Next to the calling agent's terminal.
    #[default]
    Beside,
    /// A window of its own.
    Window,
}

impl ViewerPlace {
    pub fn parse(s: &str) -> Option<ViewerPlace> {
        match s.trim().to_ascii_lowercase().as_str() {
            "beside" => Some(ViewerPlace::Beside),
            "window" => Some(ViewerPlace::Window),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            ViewerPlace::Beside => "beside",
            ViewerPlace::Window => "window",
        }
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
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub place: Option<ViewerPlace>,
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
    /// The place asked for (kept with the request).
    pub place: ViewerPlace,
    /// Where it really is: today always the agent's file viewer in that
    /// window.
    pub shown_in: String,
}

/// What the server hands the app once paths are resolved: full paths on the
/// agent's machine, checked to exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ViewerShow {
    pub agent_id: String,
    pub files: Vec<OpenedFile>,
    pub folder: Option<String>,
    pub place: ViewerPlace,
}

/// Where the app showed a [`ViewerShow`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerShown {
    /// The agent whose viewer it is.
    pub agent_id: String,
    pub window: String,
    pub in_background: bool,
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
        assert_eq!(ViewerPlace::parse(" Beside "), Some(ViewerPlace::Beside));
        assert_eq!(ViewerPlace::parse("tab"), None);
    }
}
