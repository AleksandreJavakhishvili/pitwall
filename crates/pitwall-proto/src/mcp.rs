//! The `pitwall` MCP server's tools (`show_files`, `show_folder`,
//! `show_web`): their definitions, the server's instructions, a call's
//! arguments as a [`ViewerOpen`] or a [`WebOpen`], and its result in words.
//! Which tools a Pitwall lists follows its settings ([`served`]); this
//! build's own copy, used when Pitwall can't be asked, has them all
//! ([`own`]).
//!
//! The running Pitwall serves these (`mcp.tools`, `mcp.call`), and
//! `pitwall-cli mcp` passes them through as JSON: an agent's server lives as
//! long as the agent, so one started before Pitwall was updated still gets
//! the updated tools, checked and answered by the Pitwall that shows the
//! files. The CLI uses its own build's copy only when Pitwall isn't running
//! or is older than `mcp.*`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::show::{ViewerOpen, ViewerOpened, ViewerPlace, TOOL_SHOW_FILES, TOOL_SHOW_FOLDER, TOOL_SHOW_WEB};
use crate::web::WebOpen;

/// `mcp.tools`' answer: the tools as MCP's `tools/list` gives them, the
/// server's instructions, and a stamp that changes when either does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpTools {
    #[serde(default)]
    pub tools: Value,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub stamp: String,
}

/// `mcp.call`: a `tools/call` as the agent sent it, and the agent the
/// server runs in (`$PITWALL_AGENT_ID`; the server also knows the caller).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// `mcp.call`'s answer: MCP's `tools/call` result, as it goes to the agent,
/// and the stamp of the tools the answering Pitwall has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpCalled {
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub stamp: String,
}

/// The instructions with every tool listed.
pub const INSTRUCTIONS: &str = "You are running inside Pitwall, a desktop app that shows the user their coding agents. When the user asks to see, open or look at a file, a folder or an image (\"open that file for me\", \"show me the source of worktree X\"), show it to them in Pitwall with show_files or show_folder instead of printing it. These tools only display things to the user; they don't give you the contents or any way to change files, and nothing the user does in the viewer is sent back to you. When you work in another git worktree or folder than the one you started in, give it as root (its path), so the files and their git status shown are that worktree's. When the user asks to open a web page, a site, a local dev server or a browser in Pitwall (\"open the docs on the left\", \"open a browser window\"), use show_web: it only shows the page to the user, you get nothing back from it and can't click or type in it.";

const FILES_INSTRUCTIONS: &str = "You are running inside Pitwall, a desktop app that shows the user their coding agents. When the user asks to see, open or look at a file, a folder or an image (\"open that file for me\", \"show me the source of worktree X\"), show it to them in Pitwall with show_files or show_folder instead of printing it. These tools only display things to the user; they don't give you the contents or any way to change files, and nothing the user does in the viewer is sent back to you. When you work in another git worktree or folder than the one you started in, give it as root (its path), so the files and their git status shown are that worktree's.";

const WEB_INSTRUCTIONS: &str = "You are running inside Pitwall, a desktop app that shows the user their coding agents. When the user asks to open a web page, a site, a local dev server or a browser in Pitwall (\"open the docs on the left\", \"open a browser window\"), use show_web: it only shows the page to the user, you get nothing back from it and can't click or type in it.";

/// The instructions for the tools listed.
pub fn instructions(files: bool, web: bool) -> &'static str {
    match (files, web) {
        (true, true) => INSTRUCTIONS,
        (true, false) => FILES_INSTRUCTIONS,
        (false, true) => WEB_INSTRUCTIONS,
        (false, false) => "You are running inside Pitwall, a desktop app that shows the user their coding agents. Its tools for showing the user files and web pages are turned off in Pitwall's settings.",
    }
}

/// `show_web`'s description (docs/spec/web.md quotes it).
pub const SHOW_WEB_DESCRIPTION: &str = "Show a web page to the user in Pitwall, the app you are running in: it opens in a web tile next to your terminal in Pitwall's layout (or a window of its own), in Pitwall's own web view, where the user may be signed in to sites. This only shows a page to the user: you get no page content back, and you cannot click, type, scroll or run anything in it (nothing the user does there is sent to you). Use it when the user asks to open a page, a site, a local dev server or a browser in Pitwall (\"open the docs on the left\", \"open a browser window\"), not to read the web yourself. Without url, an empty page opens with its address field for the user to type in. A later call goes to the page you opened before, wherever the user moved it, unless `where` names a place. The result says what opened and where, or why not (not a web address, Pitwall not running, turned off).";

const WHERE: &str = "Where the user sees it: right, left, above or below (a files tile in Pitwall's layout, on that side of your terminal, or of the terminal `of` names; right is the default), replace (a files tile in that terminal's place, as big and where it is: the terminal leaves the layout but keeps running, and the user can show it again from the sidebar; only when the user asks for the files instead of a terminal) or window (a separate window of its own, when the user asks for it separately). Leave it out unless the user says where: a later call then adds to the same files view wherever it is (the user may have moved it, or popped it out to a window); naming a place moves it there. The user can drag it between the layout and a window either way.";
const WEB_WHERE: &str = "Where the user sees it: right, left, above or below (a web tile in Pitwall's layout, on that side of your terminal, or of the terminal `of` names; right is the default), replace (a web tile in that terminal's place: the terminal leaves the layout but keeps running; only when the user asks for the page instead of a terminal) or window (a separate window of its own, when the user asks for one). Leave it out unless the user says where: a later call then goes to the page you opened before, wherever it is; naming a place moves it there. The user can drag it between the layout and a window either way.";
const URL: &str = "The address: http or https only (example.com/docs gets https://; localhost:3000, 127.0.0.1 and LAN hosts get http://). No file:, javascript:, data: or other schemes, and no search words. Leave it out to open an empty page with its address field for the user to type in.";
const OF: &str = "Whose terminal `where` is next to (or, with replace, in place of): another agent in the same Pitwall space, by name (\"put it above the web terminal\", \"show it instead of the web terminal\"). Default: yours.";
const ROOT: &str = "The folder relative paths are in: a path, or a worktree of this repository by its folder name or branch (to show another worktree's files). Default: the folder you work in now, as far as Pitwall can tell (else the one you started in); when you work in another worktree or folder than the one you started in, give its path here.";

/// `tools/list`'s tools, all of them.
pub fn tools() -> Value {
    tools_for(true, true)
}

/// `tools/list`'s tools: the files ones while `files`, `show_web` while
/// `web`.
pub fn tools_for(files: bool, web: bool) -> Value {
    let mut list = Vec::new();
    if files {
        list.extend(files_tools());
    }
    if web {
        list.push(json!({
            "name": TOOL_SHOW_WEB,
            "title": "Show a web page in Pitwall",
            "description": SHOW_WEB_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": URL },
                    "where": { "type": "string", "enum": ViewerPlace::WORDS, "description": WEB_WHERE },
                    "of": { "type": "string", "description": OF }
                },
                "additionalProperties": false
            },
            // A page loads in the user's browsing profile: not read-only
            // for the world, though nothing comes back to the agent.
            "annotations": { "title": "Show a web page in Pitwall", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true }
        }));
    }
    Value::Array(list)
}

fn files_tools() -> Vec<Value> {
    let annotations = |title: &str| {
        json!({ "title": title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
    };
    vec![
        json!({
            "name": TOOL_SHOW_FILES,
            "title": "Show files in Pitwall",
            "description": "Show files to the user in Pitwall, the app you are running in: each file opens as a tab in Pitwall's file viewer, a tile next to your terminal in Pitwall's layout (or a window of its own), scrolled to the line and marked when one is given. It only displays: it gives you no contents and no way to change files (the user may edit and save there themselves; nothing comes back to you). Use it when the user asks to see, open or look at a file (\"open that file for me\", \"show me where that's defined\"), not to read files yourself. Text and Markdown show as text (Markdown can be previewed), images (PNG, JPEG, GIF, WebP, SVG) as images; other files get an \"Open in default app\" button. The result says what opened and what didn't (not found, Pitwall not running, turned off).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Files to open: absolute, or relative to root. Add :line or :line:column to open at a position (src/main.rs:42)."
                    },
                    "root": { "type": "string", "description": ROOT },
                    "where": { "type": "string", "enum": ViewerPlace::WORDS, "description": WHERE },
                    "of": { "type": "string", "description": OF }
                },
                "required": ["paths"],
                "additionalProperties": false
            },
            "annotations": annotations("Show files in Pitwall")
        }),
        json!({
            "name": TOOL_SHOW_FOLDER,
            "title": "Show a folder in Pitwall",
            "description": "Show a folder to the user in Pitwall, the app you are running in: a file tree with each file's git status, in Pitwall's file viewer, a tile next to your terminal in Pitwall's layout (or a window of its own). It only displays. Any folder on this computer works, also outside your working folder. Use it when the user asks to see a folder, a project or a whole worktree (\"show me the source of worktree X\": give root X and no folder). The result says what opened, or why not.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "The folder to show: absolute, or relative to root. Leave it out to show root itself (or the folder you work in now)." },
                    "root": { "type": "string", "description": ROOT },
                    "where": { "type": "string", "enum": ViewerPlace::WORDS, "description": WHERE },
                    "of": { "type": "string", "description": OF }
                },
                "additionalProperties": false
            },
            "annotations": annotations("Show a folder in Pitwall")
        }),
    ]
}

/// This build's tools, instructions and stamp (all of them: what stands in
/// when Pitwall can't be asked).
pub fn own() -> McpTools {
    served(true, true)
}

/// The tools a Pitwall serves with showing files (`files`) and web pages
/// (`web`) on or off, its instructions and their stamp.
pub fn served(files: bool, web: bool) -> McpTools {
    let tools = tools_for(files, web);
    let instructions = instructions(files, web);
    McpTools { stamp: stamp(&tools, instructions), tools, instructions: instructions.into() }
}

/// Changes when the tools or the instructions do (FNV-1a of their JSON:
/// the same in every build).
pub fn stamp(tools: &Value, instructions: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in tools.to_string().bytes().chain([0]).chain(instructions.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// A tool call as the request it makes.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// `show_files`, `show_folder`: `viewer.open`.
    Viewer(ViewerOpen),
    /// `show_web`: `web.open`.
    Web(WebOpen),
}

/// A tool's arguments as its request (the agent's own id from its
/// environment; the server checks who calls).
pub fn request(tool: &str, args: &Value, agent: Option<String>) -> Result<Request, String> {
    let text = |k: &str| -> Result<Option<String>, String> {
        match args.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("`{k}` must be a string")),
        }
    };
    let place = match text("where")? {
        None => None,
        Some(w) => Some(ViewerPlace::parse(&w).ok_or_else(|| format!("`where` is right, left, above, below, replace or window, not {w}"))?),
    };
    if tool == TOOL_SHOW_WEB {
        let url = match text("url")? {
            None => None,
            Some(u) => Some(crate::web::address(&u)?),
        };
        return Ok(Request::Web(WebOpen { agent_id: agent, url, place, of: text("of")?, proto: Some(crate::web::WEB_PROTO) }));
    }
    let mut req = ViewerOpen { agent_id: agent, root: text("root")?, place, of: text("of")?, proto: Some(crate::show::VIEWER_PROTO), ..Default::default() };
    match tool {
        TOOL_SHOW_FILES => {
            let paths = match args.get("paths") {
                Some(Value::Array(a)) => a.iter().map(|p| p.as_str().map(String::from).ok_or("`paths` holds strings")).collect::<Result<Vec<_>, _>>()?,
                // One path given as a string: taken as it is.
                Some(Value::String(s)) => vec![s.clone()],
                _ => return Err("`paths` is required: the files to open".into()),
            };
            req.files = paths.into_iter().filter(|p| !p.trim().is_empty()).collect();
            if req.files.is_empty() {
                return Err("`paths` is empty: give the files to open".into());
            }
        }
        TOOL_SHOW_FOLDER => {
            req.folder = text("folder")?;
            if req.folder.is_none() && req.root.is_none() {
                // Nothing named: the agent's own folder.
                req.folder = Some(".".into());
            }
        }
        other => return Err(format!("no tool \"{other}\"")),
    }
    Ok(Request::Viewer(req))
}

/// What was opened, in words (the CLI's `--human` and the tools' result).
pub fn text(o: &ViewerOpened) -> String {
    let mut parts = vec![];
    for f in &o.opened {
        parts.push(match (f.line, f.column) {
            (Some(l), Some(c)) => format!("{} at line {l}, column {c}", f.path),
            (Some(l), None) => format!("{} at line {l}", f.path),
            _ => f.path.clone(),
        });
    }
    if let Some(d) = &o.folder {
        parts.push(format!("the folder {d} (as a tree)"));
    }
    let at = if o.shown_in.is_empty() { "Pitwall's file viewer" } else { o.shown_in.as_str() };
    let mut s = if parts.is_empty() { format!("Opened in {at} in Pitwall.\n") } else { format!("Opened in {at} in Pitwall: {}.\n", parts.join(", ")) };
    if let Some(n) = &o.note {
        s.push_str(n);
        s.push('\n');
    }
    if o.in_background {
        s.push_str(if o.place == ViewerPlace::Window.word() {
            "The user is in another app: the window opened behind it without taking the keyboard.\n"
        } else {
            "Pitwall isn't in front, so it opened there without coming forward; the user sees it when they switch to Pitwall.\n"
        });
    }
    if !o.skipped.is_empty() {
        let list: Vec<String> = o.skipped.iter().map(|k| format!("{} ({})", k.path, k.reason)).collect();
        s.push_str(&format!("Not opened: {}.\n", list.join(", ")));
    }
    s
}

/// `viewer.open`'s answer in words, from whatever Pitwall sent: its own
/// words (`text`) when it gave them, else read from the fields it has. It
/// is always an answer to a request Pitwall carried out, so it never turns
/// into an error, also from a newer Pitwall this build reads only in part.
pub fn opened_text(v: &Value) -> String {
    if let Some(t) = v.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
        return t.to_string();
    }
    match serde_json::from_value::<ViewerOpened>(v.clone()) {
        Ok(o) => text(&o),
        Err(_) => "Pitwall opened it, but answered in a shape this command-line tool doesn't read (Pitwall is newer than it); the user sees it in Pitwall.\n".into(),
    }
}

/// A `tools/call` result.
pub fn result(text: &str, structured: Option<Value>, error: bool) -> Value {
    let mut r = json!({ "content": [{ "type": "text", "text": text.trim_end() }], "isError": error });
    if let Some(s) = structured.filter(Value::is_object) {
        r["structuredContent"] = s;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stamp_follows_the_tools() {
        let a = own();
        assert_eq!(a.stamp, stamp(&tools(), INSTRUCTIONS));
        assert_eq!(a.stamp.len(), 16);
        assert_ne!(stamp(&json!([]), INSTRUCTIONS), a.stamp);
        assert_ne!(stamp(&a.tools, "other"), a.stamp);
    }

    /// The failure seen with a tool server from the first install (fe2fb73)
    /// talking to a later Pitwall: it read the answer into its own strict
    /// shape, so a tile that had opened was reported as an error. The
    /// answer Pitwall sends now, read by that shape and by today's.
    #[test]
    fn the_first_shape_failed_on_a_later_answer_and_this_one_doesnt() {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        enum FirstPlace {
            Beside,
            Window,
        }
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct FirstOpened {
            agent_id: String,
            agent_name: String,
            window: String,
            in_background: bool,
            opened: Vec<crate::show::OpenedFile>,
            place: FirstPlace,
            asked: Option<FirstPlace>,
            shown_in: String,
        }
        let answer = |place: &str| {
            let o = ViewerOpened {
                proto: crate::show::VIEWER_PROTO,
                agent_id: "a1".into(),
                agent_name: "api".into(),
                window: "main".into(),
                place: place.into(),
                shown_in: "a files tile in space Main (window main), right of api's terminal".into(),
                ..Default::default()
            };
            serde_json::to_value(ViewerOpened { text: Some(text(&o)), ..o }).unwrap()
        };
        let e = serde_json::from_value::<FirstOpened>(answer("right")).unwrap_err().to_string();
        assert!(e.starts_with("unknown variant `right`, expected `beside` or `window`"), "{e}");
        assert!(serde_json::from_value::<FirstOpened>(answer("window")).is_ok(), "why `where: window` worked");
        assert!(opened_text(&answer("right")).starts_with("Opened in a files tile in space Main"));
    }

    fn names(t: &Value) -> Vec<String> {
        t.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    /// `show_web` is listed while web pages may be shown, with its schema
    /// and words; the files tools while showing files is on.
    #[test]
    fn the_tools_listed_follow_the_settings() {
        assert_eq!(names(&tools()), ["show_files", "show_folder", "show_web"]);
        assert_eq!(names(&served(true, false).tools), ["show_files", "show_folder"]);
        assert_eq!(names(&served(false, true).tools), ["show_web"]);
        assert!(names(&served(false, false).tools).is_empty());
        assert_eq!(own(), served(true, true));
        // Each setting has its own stamp (the agent is told to list again).
        let stamps: std::collections::HashSet<String> = [(true, true), (true, false), (false, true), (false, false)].iter().map(|(f, w)| served(*f, *w).stamp).collect();
        assert_eq!(stamps.len(), 4);
        assert!(!served(true, false).instructions.contains("show_web"));
        assert!(served(false, true).instructions.contains("show_web") && !served(false, true).instructions.contains("show_files"));
        let web = &tools()[2];
        assert_eq!(web["description"], SHOW_WEB_DESCRIPTION);
        let d = web["description"].as_str().unwrap();
        assert!(d.contains("This only shows a page to the user: you get no page content back, and you cannot click, type"), "{d}");
        let schema = &web["inputSchema"];
        assert_eq!(schema["properties"].as_object().unwrap().keys().cloned().collect::<Vec<_>>(), ["of", "url", "where"]);
        assert_eq!(schema["properties"]["where"]["enum"], json!(ViewerPlace::WORDS));
        assert!(schema.get("required").is_none(), "url is optional");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(web["annotations"]["openWorldHint"], true);
    }

    #[test]
    fn a_show_web_call_is_checked_before_it_goes_anywhere() {
        let web = |args: Value| request(TOOL_SHOW_WEB, &args, Some("a1".into()));
        let Ok(Request::Web(r)) = web(json!({"url": "example.com/docs", "where": "left", "of": "api"})) else { panic!() };
        assert_eq!(r, WebOpen { agent_id: Some("a1".into()), url: Some("https://example.com/docs".into()), place: Some(ViewerPlace::Left), of: Some("api".into()), proto: Some(crate::web::WEB_PROTO) });
        let Ok(Request::Web(r)) = web(json!({"url": "localhost:3000", "where": "window"})) else { panic!() };
        assert_eq!((r.url.as_deref(), r.place), (Some("http://localhost:3000"), Some(ViewerPlace::Window)));
        // No url: an empty page.
        let Ok(Request::Web(r)) = web(json!({})) else { panic!() };
        assert_eq!((r.url, r.place), (None, None));
        let Ok(Request::Web(r)) = web(json!({"url": "  "})) else { panic!() };
        assert_eq!(r.url, None);
        for bad in ["javascript:alert(1)", "data:text/html,hi", "file:///etc/passwd", "about:blank", "how to cook rice", "chrome://settings"] {
            let e = web(json!({ "url": bad })).unwrap_err();
            assert!(e.contains("isn't a web address") || e.contains("Only web addresses"), "{bad}: {e}");
        }
        assert!(web(json!({"url": 3})).unwrap_err().contains("must be a string"));
        assert!(web(json!({"url": "example.com", "where": "tab"})).unwrap_err().contains("`where` is right, left"));
        assert!(matches!(request(TOOL_SHOW_FILES, &json!({"paths": ["a"]}), None), Ok(Request::Viewer(_))));
    }

    #[test]
    fn an_answer_is_read_whatever_its_shape() {
        // Pitwall's own words win.
        assert_eq!(opened_text(&json!({"text": "Opened in a tile.\n", "place": "somewhereNew"})), "Opened in a tile.\n");
        // The first shape (no words, `beside`), and a newer one (a place
        // and fields this build doesn't know): read, never an error.
        let first = json!({"agentId":"a1","agentName":"api","window":"main","inBackground":false,"opened":[{"path":"/w/a.rs","line":3}],"place":"beside","shownIn":"a pane beside api's terminal"});
        assert_eq!(opened_text(&first), "Opened in a pane beside api's terminal in Pitwall: /w/a.rs at line 3.\n");
        let newer = json!({"proto":9,"agentId":"a1","place":"floating","asked":"floating","shownIn":"a floating sheet","opened":[{"path":"/w/a.rs","extra":1}],"badge":{"x":1}});
        assert_eq!(opened_text(&newer), "Opened in a floating sheet in Pitwall: /w/a.rs.\n");
        // Even a shape that can't be read says it opened.
        assert!(opened_text(&json!({"opened": "all of it"})).starts_with("Pitwall opened it"));
    }
}
