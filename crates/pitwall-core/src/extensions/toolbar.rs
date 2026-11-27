//! `contributes.toolbar` (docs/spec/tile-sdk/ui.md §3.9): one button in
//! Pitwall's top bar, drawn by Pitwall, which an extension opts into. Its
//! icon, tooltip, maybe a badge its component sets, where it shows
//! (every space, or project spaces only), whether it starts shown, and
//! what a click does: open a tile, a popover holding a small view of one,
//! one of its commands, or a menu of up to ten.

use serde_json::Value;

use super::{Contributes, ICON_NAMES};

/// A popover's smallest and largest size (logical pixels).
pub const POPOVER_MIN: (u32, u32) = (200, 100);
pub const POPOVER_MAX: (u32, u32) = (420, 560);
/// A popover's size when the manifest names none.
pub const POPOVER_DEFAULT: (u32, u32) = (320, 400);
/// The most commands a `menu` lists.
pub const MAX_MENU: usize = 10;
/// The longest tooltip.
pub const MAX_TOOLTIP: usize = 80;

/// Where the item shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// In every space of every main window.
    #[default]
    Window,
    /// Only while the window's space is a project's.
    Project,
}

/// What a click on the item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Focus the tile where it is open, or open it.
    Tile(String),
    /// A popover under the item holding a view of the tile.
    Popover { tile: String, width: u32, height: u32 },
    /// Run one of its commands, as the palette does.
    Command(String),
    /// A menu of its commands.
    Menu(Vec<String>),
}

/// The toolbar item a manifest declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarDef {
    /// One of Pitwall's icon names.
    pub icon: String,
    pub tooltip: String,
    /// Its component sets a badge: it keeps running while no tile shows.
    pub badge: bool,
    pub scope: Scope,
    /// Shown until the user hides it (false: offered, the user turns it
    /// on).
    pub visible_by_default: bool,
    pub action: Action,
}

impl ToolbarDef {
    /// The tile its click shows, if any.
    pub fn tile(&self) -> Option<&str> {
        match &self.action {
            Action::Tile(t) | Action::Popover { tile: t, .. } => Some(t),
            _ => None,
        }
    }
}

/// `contributes.toolbar`, checked against the rest of `c` (the tile or
/// commands its action names must be there).
pub fn parse(v: &Value, c: &Contributes) -> Result<ToolbarDef, String> {
    let Value::Object(o) = v else { return Err("\"contributes.toolbar\" must be an object (one item: an extension has at most one)".into()) };
    for k in o.keys() {
        if !["icon", "tooltip", "badge", "scope", "visibleByDefault", "action"].contains(&k.as_str()) {
            return Err(format!("\"contributes.toolbar\" has “{k}”, which this Pitwall doesn't know"));
        }
    }
    let s = |k: &str| -> Result<Option<&str>, String> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(format!("\"contributes.toolbar.{k}\" must be a string")),
        }
    };
    let b = |k: &str, default: bool| -> Result<bool, String> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("\"contributes.toolbar.{k}\" must be true or false")),
        }
    };
    let icon = s("icon")?.ok_or("\"contributes.toolbar\" needs an \"icon\": one of Pitwall's icon names")?;
    if !ICON_NAMES.contains(&icon) {
        return Err(format!("\"contributes.toolbar.icon\" “{icon}” isn't one of Pitwall's icon names (an SVG of its own needs a newer Pitwall)"));
    }
    let tooltip = s("tooltip")?.ok_or("\"contributes.toolbar\" needs a \"tooltip\": what the button does, in a few words")?.trim();
    if tooltip.is_empty() || tooltip.chars().any(char::is_control) || tooltip.chars().count() > MAX_TOOLTIP {
        return Err(format!("\"contributes.toolbar.tooltip\" must be one line of at most {MAX_TOOLTIP} characters"));
    }
    let scope = match s("scope")? {
        None | Some("window") => Scope::Window,
        Some("project") => Scope::Project,
        Some(other) => return Err(format!("\"contributes.toolbar.scope\" “{other}” must be \"window\" or \"project\"")),
    };
    let Some(Value::Object(a)) = o.get("action") else { return Err("\"contributes.toolbar\" needs an \"action\": what a click does ({\"kind\": \"tile\", \"tile\": …})".into()) };
    let tile = |what: &str| -> Result<String, String> {
        let t = a.get("tile").and_then(Value::as_str).ok_or(format!("a toolbar {what} needs \"tile\": one of its tiles"))?;
        if c.tile(t).is_none() {
            return Err(format!("the toolbar's {what} names tile “{t}”, which \"contributes.tiles\" doesn't declare"));
        }
        Ok(t.to_string())
    };
    let command = |id: &str| -> Result<String, String> {
        if c.command(id).is_none() {
            return Err(format!("the toolbar names command “{id}”, which \"contributes.commands\" doesn't declare"));
        }
        Ok(id.to_string())
    };
    let action = match a.get("kind").and_then(Value::as_str) {
        Some("tile") => Action::Tile(tile("tile")?),
        Some("popover") => {
            let tile = tile("popover")?;
            let (width, height) = match a.get("size") {
                None | Some(Value::Null) => POPOVER_DEFAULT,
                Some(Value::Object(sz)) => {
                    let n = |k: &str| sz.get(k).and_then(Value::as_u64).ok_or(format!("the toolbar popover's \"size\" needs \"{k}\", a whole number of pixels"));
                    (n("w")? as u32, n("h")? as u32)
                }
                Some(_) => return Err("the toolbar popover's \"size\" must be {\"w\": …, \"h\": …}".into()),
            };
            if !(POPOVER_MIN.0..=POPOVER_MAX.0).contains(&width) || !(POPOVER_MIN.1..=POPOVER_MAX.1).contains(&height) {
                return Err(format!("the toolbar popover's size {width} × {height} must be from {} × {} to {} × {}", POPOVER_MIN.0, POPOVER_MIN.1, POPOVER_MAX.0, POPOVER_MAX.1));
            }
            Action::Popover { tile, width, height }
        }
        Some("command") => Action::Command(command(a.get("command").and_then(Value::as_str).ok_or("a toolbar command needs \"command\": one of its commands")?)?),
        Some("menu") => {
            let Some(Value::Array(list)) = a.get("commands") else { return Err("a toolbar menu needs \"commands\": a list of its commands".into()) };
            if list.is_empty() || list.len() > MAX_MENU {
                return Err(format!("a toolbar menu lists 1 to {MAX_MENU} commands"));
            }
            let ids = list.iter().map(|v| v.as_str().ok_or("a toolbar menu's \"commands\" must be command ids".to_string()).and_then(command)).collect::<Result<Vec<_>, _>>()?;
            Action::Menu(ids)
        }
        Some(other) => return Err(format!("a toolbar action's \"kind\" “{other}” must be tile, popover, command or menu")),
        None => return Err("a toolbar action needs \"kind\": tile, popover, command or menu".into()),
    };
    Ok(ToolbarDef { icon: icon.to_string(), tooltip: tooltip.to_string(), badge: b("badge", false)?, scope, visible_by_default: b("visibleByDefault", true)?, action })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{CommandContext, CommandDef, TileDef};

    fn contributes() -> Contributes {
        Contributes {
            tiles: vec![TileDef { id: "inbox".into(), title: "Inbox".into(), icon: None, wasm: "tile.wasm".into(), world: None, input: None, tools: vec![], agents: true }],
            commands: vec![CommandDef { id: "sync".into(), title: "Sync".into(), context: CommandContext::None }],
            ..Default::default()
        }
    }

    fn parse_json(s: &str) -> Result<ToolbarDef, String> {
        parse(&serde_json::from_str(s).unwrap(), &contributes())
    }

    #[test]
    fn an_item_and_its_actions() {
        let t = parse_json(r#"{"icon": "inbox", "tooltip": "Support inbox", "badge": true, "visibleByDefault": false, "action": {"kind": "popover", "tile": "inbox", "size": {"w": 360, "h": 420}}}"#).unwrap();
        assert_eq!(t.action, Action::Popover { tile: "inbox".into(), width: 360, height: 420 });
        assert!(t.badge && !t.visible_by_default && t.scope == Scope::Window);
        assert_eq!(t.tile(), Some("inbox"));
        let t = parse_json(r#"{"icon": "inbox", "tooltip": "Inbox", "scope": "project", "action": {"kind": "popover", "tile": "inbox"}}"#).unwrap();
        assert_eq!((t.action, t.scope, t.visible_by_default, t.badge), (Action::Popover { tile: "inbox".into(), width: 320, height: 400 }, Scope::Project, true, false));
        assert_eq!(parse_json(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "tile", "tile": "inbox"}}"#).unwrap().action, Action::Tile("inbox".into()));
        assert_eq!(parse_json(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "command", "command": "sync"}}"#).unwrap().action, Action::Command("sync".into()));
        assert_eq!(parse_json(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "menu", "commands": ["sync"]}}"#).unwrap().action, Action::Menu(vec!["sync".into()]));
    }

    #[test]
    fn what_check_refuses() {
        let err = |s: &str| parse_json(s).unwrap_err();
        assert!(err(r#"[{"icon": "list"}]"#).contains("at most one"));
        assert!(err(r#"{"icon": "nope", "tooltip": "x", "action": {"kind": "tile", "tile": "inbox"}}"#).contains("icon names"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "tile", "tile": "other"}}"#).contains("doesn't declare"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "command", "command": "nope"}}"#).contains("doesn't declare"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "popover", "tile": "inbox", "size": {"w": 100, "h": 300}}}"#).contains("from 200 × 100 to 420 × 560"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "popover", "tile": "inbox", "size": {"w": 300, "h": 600}}}"#).contains("must be from"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "menu", "commands": []}}"#).contains("1 to 10"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "action": {"kind": "fly"}}"#).contains("tile, popover, command or menu"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "scope": "space", "action": {"kind": "tile", "tile": "inbox"}}"#).contains("window"));
        assert!(err(r#"{"icon": "list", "tooltip": "x", "colour": "red", "action": {"kind": "tile", "tile": "inbox"}}"#).contains("colour"));
        assert!(err(&format!(r#"{{"icon": "list", "tooltip": "{}", "action": {{"kind": "tile", "tile": "inbox"}}}}"#, "x".repeat(81))).contains("80"));
    }
}
