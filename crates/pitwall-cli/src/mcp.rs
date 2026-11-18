//! `pitwall mcp`: Pitwall's MCP server, on stdin/stdout (JSON-RPC 2.0, one
//! message per line, as MCP's stdio transport says). Pitwall adds it to
//! the agents it starts (`agents.showFiles`) so they can show the user
//! files; the tools' descriptions tell the agent when to use them. Each
//! call is `pitwall open`'s request ([`crate::open`]), sent to the running
//! Pitwall on a connection of its own, so a Pitwall started after the
//! agent still answers. Nothing is ever printed but protocol messages.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use pitwall_client::{Client, Error};
use pitwall_proto::show::{MCP_SERVER, TOOL_SHOW_FILES, TOOL_SHOW_FOLDER};
use pitwall_proto::{code, ViewerOpen, ViewerOpened, ViewerPlace};

/// Protocol versions this server speaks, newest first.
pub const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// What a tool call does: `viewer.open` on the running Pitwall (tests plug
/// in their own).
pub trait Opener {
    fn open(&self, req: &ViewerOpen) -> Result<ViewerOpened, Error>;
}

/// The running Pitwall at `socket`.
pub struct SocketOpener {
    pub socket: PathBuf,
}

impl Opener for SocketOpener {
    fn open(&self, req: &ViewerOpen) -> Result<ViewerOpened, Error> {
        let mut c = Client::connect_as(&self.socket, &format!("pitwall-mcp/{}", env!("CARGO_PKG_VERSION")), pitwall_proto::Role::Cli)?;
        crate::open::open(&mut c, req)
    }
}

const INSTRUCTIONS: &str = "You are running inside Pitwall, a desktop app that shows the user their coding agents. When the user asks to see, open or look at a file, a folder or an image (\"open that file for me\", \"show me the source of worktree X\"), show it to them in Pitwall with show_files or show_folder instead of printing it. These tools only display things to the user; they don't give you the contents or any way to change files, and nothing the user does in the viewer is sent back to you.";

const WHERE: &str = "Where the user sees it: window (a separate window of its own, the default) or beside (a pane split next to your terminal, in the Pitwall window that shows you). Leave it out unless the user says where: it then opens in a window, or beside the terminal if the user docked it there. A later call adds to the same pane or window.";
const ROOT: &str = "The folder relative paths are in: a path, or a worktree of this repository by its folder name or branch (to show another worktree's files). Default: your working folder.";

/// `tools/list`'s tools.
pub fn tools() -> Value {
    let annotations = |title: &str| {
        json!({ "title": title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false })
    };
    json!([
        {
            "name": TOOL_SHOW_FILES,
            "title": "Show files in Pitwall",
            "description": "Show files to the user in Pitwall, the app you are running in: each file opens as a tab in Pitwall's file viewer next to your terminal (or in a window of its own), scrolled to the line and marked when one is given. It only displays: it gives you no contents and no way to change files (the user may edit and save there themselves; nothing comes back to you). Use it when the user asks to see, open or look at a file (\"open that file for me\", \"show me where that's defined\"), not to read files yourself. Text and Markdown show as text (Markdown can be previewed), images (PNG, JPEG, GIF, WebP, SVG) as images; other files get an \"Open in default app\" button. The result says what opened and what didn't (not found, Pitwall not running, turned off).",
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
                    "where": { "type": "string", "enum": ["beside", "window"], "description": WHERE }
                },
                "required": ["paths"],
                "additionalProperties": false
            },
            "annotations": annotations("Show files in Pitwall")
        },
        {
            "name": TOOL_SHOW_FOLDER,
            "title": "Show a folder in Pitwall",
            "description": "Show a folder to the user in Pitwall, the app you are running in: a file tree with each file's git status, in Pitwall's file viewer next to your terminal (or in a window of its own). It only displays. Any folder on this computer works, also outside your working folder. Use it when the user asks to see a folder, a project or a whole worktree (\"show me the source of worktree X\": give root X and no folder). The result says what opened, or why not.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "The folder to show: absolute, or relative to root. Leave it out to show root itself (or your working folder)." },
                    "root": { "type": "string", "description": ROOT },
                    "where": { "type": "string", "enum": ["beside", "window"], "description": WHERE }
                },
                "additionalProperties": false
            },
            "annotations": annotations("Show a folder in Pitwall")
        }
    ])
}

/// A tool's arguments as `viewer.open`'s request (the agent's own id from
/// its environment; the server checks who calls).
pub fn request(tool: &str, args: &Value, agent: Option<String>) -> Result<ViewerOpen, String> {
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
        Some(w) => Some(ViewerPlace::parse(&w).ok_or_else(|| format!("`where` is beside or window, not {w}"))?),
    };
    let mut req = ViewerOpen { agent_id: agent, root: text("root")?, place, ..Default::default() };
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
    Ok(req)
}

/// What a failed call tells the agent.
fn failure(e: &Error) -> String {
    match e {
        Error::Connect { .. } => "Pitwall isn't running (or can't be reached), so nothing was opened.".into(),
        Error::Server(b) if b.code == code::OFF => b.message.clone(),
        Error::Server(b) => format!("Nothing was opened: {}", b.message),
        other => format!("Nothing was opened: {other}"),
    }
}

/// `tools/call`'s result.
pub fn call(opener: &dyn Opener, params: &Value, agent: Option<String>) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let res = request(name, &args, agent).map_err(|e| format!("Nothing was opened: {e}")).and_then(|req| opener.open(&req).map_err(|e| failure(&e)));
    match res {
        Ok(o) => json!({
            "content": [{ "type": "text", "text": crate::open::text(&o).trim_end() }],
            "structuredContent": o,
            "isError": false
        }),
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
    }
}

/// The answer to one message (`None`: a notification, or a response).
pub fn handle(opener: &dyn Opener, msg: &Value, agent: Option<String>) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id, method) else {
        // A notification (`notifications/initialized`, …) or a reply to
        // something this server never asks: nothing to say.
        return None;
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": MCP_SERVER, "title": "Pitwall", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(opener, &params, agent),
        other => {
            return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {other}") } }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Serve until stdin closes.
pub fn serve(opener: &dyn Opener, input: impl BufRead, out: &mut impl Write) -> std::io::Result<()> {
    let agent = crate::open::env_agent();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) if msg.is_object() => handle(opener, &msg, agent.clone()),
            Ok(_) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "one JSON-RPC request per line" } })),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } })),
        };
        if let Some(r) = reply {
            writeln!(out, "{r}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use pitwall_proto::{ErrorBody, OpenedFile};

    /// Remembers the requests; answers with `answer`.
    struct Fake {
        seen: RefCell<Vec<ViewerOpen>>,
        answer: fn(&ViewerOpen) -> Result<ViewerOpened, Error>,
    }

    fn opened(req: &ViewerOpen) -> Result<ViewerOpened, Error> {
        Ok(ViewerOpened {
            agent_id: "a1".into(),
            agent_name: "api".into(),
            window: "main".into(),
            in_background: false,
            opened: req.files.iter().map(|f| OpenedFile { path: format!("/w/{f}"), line: None, column: None }).collect(),
            folder: req.folder.clone(),
            skipped: vec![],
            place: req.place.unwrap_or_default(),
            asked: req.place,
            shown_in: match req.place {
                Some(ViewerPlace::Window) => "a file viewer window of its own for api, showing /w".into(),
                _ => "a file viewer beside api's terminal (window main), showing /w".into(),
            },
            note: None,
        })
    }

    impl Opener for Fake {
        fn open(&self, req: &ViewerOpen) -> Result<ViewerOpened, Error> {
            self.seen.borrow_mut().push(req.clone());
            (self.answer)(req)
        }
    }

    fn fake(answer: fn(&ViewerOpen) -> Result<ViewerOpened, Error>) -> Fake {
        Fake { seen: RefCell::new(vec![]), answer }
    }

    /// Lines in, lines out, as a client sees it over stdio.
    fn session(opener: &dyn Opener, lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let mut out = Vec::new();
        serve(opener, input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn initialize_list_and_call_over_stdio() {
        let f = fake(opened);
        let out = session(
            &f,
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"show_files","arguments":{"paths":["src/a.rs:12","README.md"],"where":"window"}}}),
                json!({"jsonrpc":"2.0","id":"x","method":"ping"}),
            ],
        );
        assert_eq!(out.len(), 4, "the notification gets no answer: {out:?}");
        assert_eq!(out[0]["id"], 1);
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "pitwall");
        assert!(out[0]["result"]["capabilities"]["tools"].is_object());
        let tools = out[1]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["show_files", "show_folder"]);
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object");
            assert_eq!(t["annotations"]["readOnlyHint"], true, "Codex runs read-only tools without asking");
            assert!(t["description"].as_str().unwrap().contains("Pitwall"));
        }
        assert_eq!(tools[0]["inputSchema"]["required"], json!(["paths"]));
        let r = &out[2]["result"];
        assert_eq!(r["isError"], false);
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("Opened in a file viewer window of its own for api, showing /w in Pitwall: /w/src/a.rs:12, /w/README.md."));
        assert_eq!(r["structuredContent"]["place"], "window");
        assert_eq!(out[3], json!({"jsonrpc":"2.0","id":"x","result":{}}));
        let seen = f.seen.borrow();
        assert_eq!(seen[0].files, ["src/a.rs:12", "README.md"]);
        assert_eq!(seen[0].place, Some(ViewerPlace::Window));
        assert_eq!(seen[0].cwd, None, "relative paths are the agent's folder's");
    }

    #[test]
    fn an_unknown_version_gets_the_newest() {
        let out = session(&fake(opened), &[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}})]);
        assert_eq!(out[0]["result"]["protocolVersion"], VERSIONS[0]);
    }

    #[test]
    fn folders_default_to_the_root_or_the_agents_folder() {
        let f = fake(opened);
        let call = |args: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"show_folder","arguments":args}});
        session(&f, &[call(json!({})), call(json!({"root":"fix/login"})), call(json!({"folder":"src"}))]);
        let seen = f.seen.borrow();
        assert_eq!((seen[0].folder.as_deref(), seen[0].root.as_deref()), (Some("."), None));
        assert_eq!((seen[1].folder.as_deref(), seen[1].root.as_deref()), (None, Some("fix/login")));
        assert_eq!(seen[2].folder.as_deref(), Some("src"));
    }

    #[test]
    fn failures_are_tool_errors_that_say_why() {
        let call = |name: &str, args: Value| json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":args}});
        let not_running = fake(|_| Err(Error::Connect { path: "/x.sock".into(), source: std::io::Error::from(std::io::ErrorKind::NotFound) }));
        let off = fake(|_| Err(Error::Server(ErrorBody::new(code::OFF, pitwall_daemon::viewer::OFF))));
        let missing = fake(|_| Err(Error::Server(ErrorBody::new(code::NOT_FOUND, "nothing was opened: gone.rs (not found)"))));
        let text = |f: &Fake, c: Value| {
            let out = session(f, &[c]);
            assert_eq!(out[0]["result"]["isError"], true, "{out:?}");
            out[0]["result"]["content"][0]["text"].as_str().unwrap().to_string()
        };
        assert!(text(&not_running, call("show_files", json!({"paths":["a"]}))).contains("Pitwall isn't running"));
        assert!(text(&off, call("show_files", json!({"paths":["a"]}))).contains("turned off"));
        assert!(text(&missing, call("show_files", json!({"paths":["gone.rs"]}))).contains("gone.rs (not found)"));
        let ok = fake(opened);
        assert!(text(&ok, call("show_files", json!({}))).contains("`paths` is required"));
        assert!(text(&ok, call("show_files", json!({"paths":[]}))).contains("empty"));
        assert!(text(&ok, call("show_files", json!({"paths":["a"],"where":"tab"}))).contains("beside or window"));
        assert!(text(&ok, call("rm_rf", json!({}))).contains("no tool"));
        assert!(ok.seen.borrow().is_empty(), "nothing bad reaches Pitwall");
    }

    #[test]
    fn bad_lines_and_unknown_methods() {
        let input = "not json\n[1]\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"resources/list\"}\n";
        let mut out = Vec::new();
        serve(&fake(opened), input.as_bytes(), &mut out).unwrap();
        let out: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(out[0]["error"]["code"], -32700);
        assert_eq!(out[1]["error"]["code"], -32600);
        assert_eq!((out[2]["id"].clone(), out[2]["error"]["code"].clone()), (json!(5), json!(-32601)));
    }
}
