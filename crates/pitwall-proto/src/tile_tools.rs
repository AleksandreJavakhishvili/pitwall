//! Extensions' tile tools in the agents' `pitwall` MCP server
//! (docs/spec/tile-sdk/agent-tools.md §11.3–11.8): the open Pitwall gives
//! every extension with tiles (`<extension-id>__open`) and the show tools
//! a tile declares (`<extension-id>__<tool>`), listed beside `show_files`
//! and the rest for every agent Pitwall starts with that server.
//!
//! The app lists them ([`TileTool`]), the server turns each into an MCP
//! tool ([`mcp_tool`]: its schema with Pitwall's `where`, `of` and `item`
//! added, `readOnlyHint`), reads a call's arguments ([`parse`]) and asks
//! the app to show it ([`TileShow`] → [`TileShown`]).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::show::ViewerPlace;

/// A tile tool, as the running Pitwall lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TileTool {
    /// The extension's id.
    pub extension: String,
    /// Its name, as the user sees it ("Diagrams").
    pub extension_name: String,
    /// What the extension is, in one line (its manifest's description).
    #[serde(default)]
    pub summary: String,
    /// The declared tool's name; `None`: the open every tile has.
    #[serde(default)]
    pub tool: Option<String>,
    /// `show` or `present`.
    pub kind: String,
    /// A declared tool's description (the open's is Pitwall's).
    #[serde(default)]
    pub description: String,
    /// A declared tool's input schema.
    #[serde(default)]
    pub input: Option<Value>,
    /// The tiles it opens: the declared tool's own, or every tile agents
    /// may open (for the open).
    pub tiles: Vec<TileTarget>,
}

/// One of an extension's tiles, as a tile tool opens it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TileTarget {
    pub id: String,
    pub title: String,
    /// The content it takes when opened (its `"input"` schema).
    #[serde(default)]
    pub input: Option<Value>,
}

/// The longest tile tool name: `mcp__pitwall__<name>` within the 64
/// characters Claude Code and Codex take (the manifest check refuses
/// longer ones; this keeps one out of the list anyway).
pub const MAX_NAME: usize = 64 - "mcp__pitwall__".len();

/// A tile tool's name in MCP: `<extension-id>__<tool>`, the id's `.`
/// as `-` (`diagrams__show_diagram`, `diagrams__open`).
pub fn mcp_name(extension: &str, tool: Option<&str>) -> String {
    format!("{}__{}", extension.replace('.', "-"), tool.unwrap_or("open"))
}

impl TileTool {
    /// Its name in MCP ([`mcp_name`]).
    pub fn name(&self) -> String {
        mcp_name(&self.extension, self.tool.as_deref())
    }

    /// Whether it is a `present` tool.
    pub fn presents(&self) -> bool {
        self.kind == "present"
    }

    /// The schema its content is checked against for tile `tile`.
    pub fn content_schema(&self, tile: &str) -> Option<&Value> {
        match &self.tool {
            Some(_) => self.input.as_ref(),
            None => self.tiles.iter().find(|t| t.id == tile).and_then(|t| t.input.as_ref()),
        }
    }
}

const WHERE: &str = "Where the user sees the tile: right, left, above or below (in Pitwall's layout, on that side of your terminal, or of the terminal `of` names; right is the default), replace (in that terminal's place: the terminal leaves the layout but keeps running; only when the user asks) or window (a window of its own, when the user asks). Leave it out unless the user says where: a later call then goes to the tile where it is (the user may have moved it); naming a place moves it there.";
const OF: &str = "Whose terminal `where` is next to: another agent in the same Pitwall space, by name. Default: yours.";
const ITEM: &str = "Your id for what you show (\"auth-flow\"): a later call with the same item updates it in place, a new item adds one. Items are yours: other agents' items with the same id are theirs. When the user has changed an item you showed, a call with its id is refused: show it under a new id.";

/// The open's or tool's description, as agents read it.
pub fn description(t: &TileTool) -> String {
    let name = &t.extension_name;
    match &t.tool {
        Some(_) => format!("{name} (a Pitwall tile): {}", t.description.trim()),
        None => {
            let what = if t.tiles.len() == 1 { format!("the {name} tile") } else { format!("a {name} tile (`tile` says which)") };
            let content = if t.tiles.iter().any(|x| x.input.is_some()) {
                " with the content you give it, which only displays it to the user (you get nothing of the tile back)"
            } else {
                ""
            };
            let summary = if t.summary.trim().is_empty() { String::new() } else { format!(" {name}: {}.", t.summary.trim().trim_end_matches('.')) };
            format!("{name} (a Pitwall tile): open {what} in Pitwall, the app you are running in, next to your terminal{content}.{summary} Use it when the user asks to see something there. The result says what opened and where, or what is wrong with the input.")
        }
    }
}

/// `schema`'s properties (an object schema), else none.
fn properties(schema: Option<&Value>) -> Map<String, Value> {
    schema.and_then(|s| s.get("properties")).and_then(Value::as_object).cloned().unwrap_or_default()
}

fn is_object_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
}

/// The MCP input schema: the content's own properties (a multi-tile
/// open: `tile` and `content`), then `item`, `where` and `of`.
pub fn input_schema(t: &TileTool) -> Value {
    let mut props = Map::new();
    let mut required: Vec<Value> = Vec::new();
    let mut takes_content = true;
    match (&t.tool, t.tiles.as_slice()) {
        (Some(_), _) => {
            props = properties(t.input.as_ref());
            required = t.input.as_ref().and_then(|s| s.get("required")).and_then(Value::as_array).cloned().unwrap_or_default();
        }
        (None, [one]) => match &one.input {
            Some(s) if is_object_schema(s) => {
                props = properties(Some(s));
                required = s.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
            }
            Some(s) => {
                props.insert("content".into(), s.clone());
                required.push("content".into());
            }
            None => takes_content = false,
        },
        (None, tiles) => {
            let ids: Vec<&str> = tiles.iter().map(|x| x.id.as_str()).collect();
            let list: Vec<String> = tiles.iter().map(|x| format!("{} ({})", x.id, x.title)).collect();
            props.insert("tile".into(), json!({ "type": "string", "enum": ids, "description": format!("Which tile: {}. Default: {}.", list.join(", "), ids.first().copied().unwrap_or("")) }));
            let mut what = Vec::new();
            for x in tiles {
                match &x.input {
                    Some(s) => what.push(format!("{}: {}", x.id, s)),
                    None => what.push(format!("{}: takes none", x.id)),
                }
            }
            props.insert("content".into(), json!({ "type": "object", "description": format!("What to show, as that tile's schema says ({})", what.join("; ")) }));
            takes_content = tiles.iter().any(|x| x.input.is_some());
        }
    }
    if takes_content && !props.contains_key("item") {
        props.insert("item".into(), json!({ "type": "string", "maxLength": 200, "description": ITEM }));
    }
    props.insert("where".into(), json!({ "type": "string", "enum": ViewerPlace::WORDS, "description": WHERE }));
    props.insert("of".into(), json!({ "type": "string", "description": OF }));
    let mut s = json!({ "type": "object", "properties": props });
    if !required.is_empty() {
        s["required"] = Value::Array(required);
    }
    s
}

/// The tool as MCP's `tools/list` gives it: read-only (it changes nothing
/// outside the tile, so agents that ask per tool don't).
pub fn mcp_tool(t: &TileTool) -> Value {
    let title = match &t.tool {
        Some(name) => format!("{}: {name}", t.extension_name),
        None => format!("Open {} in Pitwall", t.extension_name),
    };
    json!({
        "name": t.name(),
        "title": title,
        "description": description(t),
        "inputSchema": input_schema(t),
        "annotations": { "title": title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
    })
}

/// The servers' instructions for tile tools: one line per extension,
/// within a fixed budget.
pub fn instructions(tools: &[TileTool]) -> String {
    const BUDGET: usize = 1500;
    let mut seen: Vec<&str> = Vec::new();
    let mut lines = String::new();
    for t in tools {
        if seen.contains(&t.extension.as_str()) {
            continue;
        }
        seen.push(&t.extension);
        let names: Vec<String> = tools.iter().filter(|x| x.extension == t.extension).map(TileTool::name).collect();
        let summary = if t.summary.trim().is_empty() { String::new() } else { format!(": {}", t.summary.trim().trim_end_matches('.')) };
        let line = format!(" {}{summary} ({}).", t.extension_name, names.join(", "));
        if lines.len() + line.len() > BUDGET {
            lines.push_str(" …and more: see the tools list.");
            break;
        }
        lines.push_str(&line);
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(" Extensions' tiles in Pitwall have tools too, named <extension>__<tool>: each only shows something to the user in a tile next to your terminal and gives you nothing of the tile back.{lines}")
}

/// A tile tool call, read from its arguments.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TileCall {
    pub tile: String,
    pub item: Option<String>,
    /// The content for the tile (without Pitwall's own arguments).
    pub content: Option<Value>,
    pub place: Option<ViewerPlace>,
    pub of: Option<String>,
}

/// Read a call of `t` with `args`: Pitwall's own arguments taken out, the
/// rest the content.
pub fn parse(t: &TileTool, args: &Value) -> Result<TileCall, String> {
    let empty = Map::new();
    let a = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        _ => return Err("the arguments must be an object".into()),
    };
    let text = |k: &str| -> Result<Option<String>, String> {
        match a.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
            Some(_) => Err(format!("`{k}` must be a string")),
        }
    };
    let place = match text("where")? {
        None => None,
        Some(w) => Some(ViewerPlace::parse(&w).ok_or_else(|| format!("`where` is right, left, above, below, replace or window, not {w}"))?),
    };
    let of = text("of")?;
    let item = text("item")?;
    if item.as_ref().is_some_and(|i| i.chars().count() > 200) {
        return Err("`item` is longer than 200 characters".into());
    }
    let multi = t.tool.is_none() && t.tiles.len() > 1;
    let tile = if multi {
        match text("tile")? {
            Some(id) if t.tiles.iter().any(|x| x.id == id) => id,
            Some(id) => return Err(format!("`tile` is one of {}, not {id}", t.tiles.iter().map(|x| x.id.as_str()).collect::<Vec<_>>().join(", "))),
            None => t.tiles.first().map(|x| x.id.clone()).unwrap_or_default(),
        }
    } else {
        t.tiles.first().map(|x| x.id.clone()).unwrap_or_default()
    };
    let schema = t.content_schema(&tile);
    // The item stays in the content when the schema declares it.
    let keeps_item = properties(schema).contains_key("item");
    let content = if multi {
        a.get("content").cloned().filter(|c| !c.is_null())
    } else if schema.is_some_and(|s| !is_object_schema(s)) {
        a.get("content").cloned()
    } else if schema.is_none() && t.tool.is_none() {
        None
    } else {
        let mut rest = a.clone();
        for k in ["where", "of"] {
            rest.remove(k);
        }
        if !keeps_item {
            rest.remove("item");
        }
        Some(Value::Object(rest))
    };
    Ok(TileCall { tile, item, content, place, of })
}

/// `tile.show`: what the server asks the app to show, the agent resolved
/// and the input checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TileShow {
    /// The agent that called (its terminal is where the tile goes).
    pub agent_id: String,
    pub agent_name: String,
    /// Its project folder.
    #[serde(default)]
    pub project: Option<String>,
    pub extension: String,
    pub tile: String,
    /// The declared tool; `None`: the open.
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub item: Option<String>,
    /// The content, as JSON text.
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub place: Option<ViewerPlace>,
    /// The agent whose terminal `place` is next to (resolved id).
    #[serde(default)]
    pub of: Option<String>,
}

/// What the tile answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TileReply {
    /// It showed it (false: it changed nothing, `text` says why).
    pub ok: bool,
    pub text: String,
    #[serde(default)]
    pub json: Option<String>,
    #[serde(default)]
    pub item: Option<String>,
}

/// Where the app showed a [`TileShow`], and the tile's reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TileShown {
    pub window: String,
    pub in_background: bool,
    pub place: ViewerPlace,
    /// Where it is, in words.
    pub at: String,
    #[serde(default)]
    pub note: Option<String>,
    /// The tile was open already (the call went to it).
    #[serde(default)]
    pub reused: bool,
    /// The tile's title.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reply: Option<TileReply>,
}

/// What a call did, in words for the agent (and whether it is an error).
pub fn text(t: &TileTool, req: &TileShow, s: &TileShown) -> (String, bool) {
    let title = if s.title.is_empty() { t.extension_name.clone() } else { s.title.clone() };
    let mut out = String::new();
    let failed = s.reply.as_ref().is_some_and(|r| !r.ok);
    if failed {
        out.push_str(s.reply.as_ref().map(|r| r.text.as_str()).unwrap_or_default());
        out.push('\n');
        return (out, true);
    }
    let what = match (&req.item, s.reused) {
        (Some(i), true) => format!("Showed “{i}” in {title}, which was open"),
        (Some(i), false) => format!("Opened {title} with “{i}”"),
        (None, true) => format!("Showed it in {title}, which was open"),
        (None, false) => format!("Opened {title}"),
    };
    out.push_str(&format!("{what}, in {} in Pitwall.\n", s.at));
    if let Some(r) = s.reply.as_ref().filter(|r| !r.text.trim().is_empty()) {
        out.push_str(r.text.trim());
        out.push('\n');
    }
    if let Some(n) = &s.note {
        out.push_str(n);
        out.push('\n');
    }
    if s.in_background {
        out.push_str("Pitwall isn't in front, so it opened there without coming forward; the user sees it when they switch to Pitwall.\n");
    }
    out.push_str("You get nothing of the tile back; the user may change what you showed.\n");
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> TileTool {
        TileTool {
            extension: "hello".into(),
            extension_name: "Hello".into(),
            summary: "A counter and a greeting".into(),
            tool: None,
            kind: "show".into(),
            tiles: vec![TileTarget { id: "main".into(), title: "Hello".into(), input: Some(json!({ "type": "object", "properties": { "message": { "type": "string" } } })) }],
            ..Default::default()
        }
    }

    #[test]
    fn names_follow_the_extension() {
        assert_eq!(mcp_name("hello", None), "hello__open");
        assert_eq!(mcp_name("acme.diagrams", Some("show_diagram")), "acme-diagrams__show_diagram");
        // What every MCP kind takes: letters, digits, `_` and `-`, at most
        // 64 with the server's prefix (Claude Code's `mcp__pitwall__…`,
        // Codex's and Qwen's the same; Copilot and Amp no stricter).
        let longest = mcp_name(&"a".repeat(MAX_NAME - "__show".len()), Some("show"));
        assert_eq!(longest.len(), MAX_NAME);
        let qualified = format!("mcp__pitwall__{longest}");
        assert_eq!(qualified.len(), 64);
        assert!(qualified.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        assert!(mcp_name("a.b-c", Some("x_1")).chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn the_open_takes_the_tiles_input_and_pitwalls_own() {
        let t = mcp_tool(&hello());
        assert_eq!(t["name"], "hello__open");
        assert_eq!(t["annotations"]["readOnlyHint"], true);
        let props = t["inputSchema"]["properties"].as_object().unwrap();
        assert!(["message", "item", "where", "of"].iter().all(|k| props.contains_key(*k)), "{props:?}");
        assert!(t["description"].as_str().unwrap().starts_with("Hello (a Pitwall tile): open the Hello tile"));
        let c = parse(&hello(), &json!({ "message": "hi", "item": "greeting", "where": "left", "of": "web" })).unwrap();
        assert_eq!(c, TileCall { tile: "main".into(), item: Some("greeting".into()), content: Some(json!({ "message": "hi" })), place: Some(ViewerPlace::Left), of: Some("web".into()) });
        assert!(parse(&hello(), &json!({ "where": "up" })).unwrap_err().contains("`where`"));
    }

    #[test]
    fn several_tiles_take_tile_and_content() {
        let mut t = hello();
        t.tiles.push(TileTarget { id: "second".into(), title: "Hello again".into(), input: None });
        let s = input_schema(&t);
        assert_eq!(s["properties"]["tile"]["enum"], json!(["main", "second"]));
        let c = parse(&t, &json!({ "tile": "main", "content": { "message": "x" } })).unwrap();
        assert_eq!((c.tile.as_str(), c.content), ("main", Some(json!({ "message": "x" }))));
        assert!(parse(&t, &json!({ "tile": "third" })).is_err());
    }

    #[test]
    fn a_declared_tool_keeps_a_declared_item() {
        let t = TileTool { tool: Some("show_diagram".into()), kind: "present".into(), description: "Show a flowchart.".into(), input: Some(json!({ "type": "object", "required": ["mermaid"], "properties": { "item": { "type": "string" }, "mermaid": { "type": "string" } } })), ..hello() };
        assert_eq!(t.name(), "hello__show_diagram");
        assert_eq!(mcp_tool(&t)["inputSchema"]["required"], json!(["mermaid"]));
        let c = parse(&t, &json!({ "item": "a", "mermaid": "flowchart LR" })).unwrap();
        assert_eq!(c.content, Some(json!({ "item": "a", "mermaid": "flowchart LR" })));
        assert!(instructions(&[hello(), t]).contains("Hello: A counter and a greeting (hello__open, hello__show_diagram)."));
        assert_eq!(instructions(&[]), "");
    }
}
