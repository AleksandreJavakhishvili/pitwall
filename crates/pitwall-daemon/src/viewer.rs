//! `viewer.open`: show files and folders to the user in an agent's file
//! viewer (`pitwall open`, and the `pitwall` MCP server agents get). The
//! server checks the setting, finds the agent and resolves the paths on its
//! machine (`pitwall_core::show`); the host's [`ViewerBackend`] shows them
//! in the right window. Nothing here asks the user: it only shows things,
//! read-only, in Pitwall.
//!
//! The agents' `pitwall` MCP tools are served here too (`mcp.tools`,
//! `mcp.call`): their MCP servers outlive updates, so they pass the tools
//! through instead of carrying their own.

use serde_json::Value;

use pitwall_proto::mcp::{self, McpCall, McpCalled, McpTools};
use pitwall_proto::{code, ErrorBody, ViewerOpen, ViewerOpened, ViewerPlace, ViewerShow, ViewerShown, VIEWER_PROTO};

use crate::identity::Caller;
use crate::server::Server;

/// The host's windows. Called on connection threads; may block while the
/// host's UI thread opens the viewer.
pub trait ViewerBackend: Send + Sync {
    /// Open `req` (paths already resolved and checked) for `req.agent_id`:
    /// a files tile next to its terminal's slot in the layout of the window
    /// that shows it, or a window
    /// of its own.
    fn show(&self, req: &ViewerShow) -> Result<ViewerShown, ErrorBody>;
    /// The name of agent `id` when it is one shown here from another
    /// machine (a peer's), which has no files here.
    fn mirrored(&self, id: &str) -> Option<String>;
}

type Res = Result<Value, ErrorBody>;

/// What to say when `agents.showFiles` is off.
pub const OFF: &str = "Showing files is turned off in Pitwall (Settings → Agents → “Agents can show you files”, or `pitwall settings set agents.showFiles true`). Nothing was opened.";

/// `viewer.open`'s params, read: one this Pitwall can't read because the
/// asker is newer (a `where` it doesn't know yet) says so plainly.
pub fn params(p: Value) -> Result<ViewerOpen, ErrorBody> {
    let newer = p.get("proto").and_then(Value::as_u64).is_some_and(|v| v > u64::from(VIEWER_PROTO));
    serde_json::from_value(p).map_err(|e| {
        if newer {
            ErrorBody::new(code::BAD_PARAMS, format!("This Pitwall is older than its command-line tool ({e}): quit Pitwall and open it again to update it. Nothing was opened."))
        } else {
            ErrorBody::new(code::BAD_PARAMS, e.to_string())
        }
    })
}

pub fn open(s: &Server, caller: &Caller, r: ViewerOpen) -> Res {
    let first = r.proto.is_none();
    let mut o = show(s, caller, r)?;
    if first {
        // Asked in the first shape (a CLI or an agent's tool server from
        // before `proto`, still running): answered in its words, which it
        // reads strictly (`beside` or `window`), or it would report what
        // opened as an error.
        fn word(p: &str) -> String {
            let w = if p == ViewerPlace::Window.word() { "window" } else { "beside" };
            w.to_string()
        }
        o.place = word(&o.place);
        o.asked = o.asked.as_deref().map(word);
    }
    serde_json::to_value(o).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// `mcp.tools`: this Pitwall's tools.
pub fn mcp_tools() -> Res {
    serde_json::to_value(mcp::own()).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// `mcp.call`: a tool call, checked against this Pitwall's tools and
/// answered as the agent sees it (a failure is a tool error that says why).
pub fn mcp_call(s: &Server, caller: &Caller, c: McpCall) -> Res {
    let result = match mcp::request(&c.name, &c.arguments, c.agent.clone()) {
        Err(e) => mcp::result(&format!("Nothing was opened: {e}"), None, true),
        Ok(req) => match show(s, caller, req) {
            Ok(o) => {
                let text = o.text.clone().unwrap_or_else(|| mcp::text(&o));
                mcp::result(&text, serde_json::to_value(&o).ok(), false)
            }
            Err(e) if e.code == code::OFF => mcp::result(&e.message, None, true),
            Err(e) => mcp::result(&format!("Nothing was opened: {}", e.message), None, true),
        },
    };
    let McpTools { stamp, .. } = mcp::own();
    serde_json::to_value(McpCalled { result, stamp }).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

fn show(s: &Server, caller: &Caller, r: ViewerOpen) -> Result<ViewerOpened, ErrorBody> {
    let Some(backend) = s.viewer.as_deref() else {
        return Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall has no windows to show files in"));
    };
    if !s.engine.show_files_on() {
        return Err(ErrorBody::new(code::OFF, OFF));
    }
    if r.files.iter().all(|f| f.trim().is_empty()) && r.folder.as_deref().is_none_or(|f| f.trim().is_empty()) && r.root.is_none() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "nothing to show: give files or a folder"));
    }
    let key = r
        .agent_id
        .clone()
        .filter(|a| !a.trim().is_empty())
        .or_else(|| caller.agent.as_ref().map(|(id, _)| id.clone()))
        .ok_or_else(|| ErrorBody::new(code::BAD_PARAMS, "say whose viewer (--agent NAME): this doesn't run inside an agent Pitwall started"))?;
    let key = key.trim();
    let views = s.engine.views();
    let agent = views.iter().find(|v| v.id == key).or_else(|| {
        let named: Vec<_> = views.iter().filter(|v| v.name.eq_ignore_ascii_case(key)).collect();
        (named.len() == 1).then(|| named[0])
    });
    let Some(agent) = agent else {
        if let Some(name) = backend.mirrored(key) {
            return Err(ErrorBody::new(
                code::UNSUPPORTED,
                format!("{name} runs on another machine and is only shown here, read-only: Pitwall can't show its files"),
            ));
        }
        return Err(ErrorBody::new(code::NOT_FOUND, format!("no agent \"{key}\" in Pitwall (see `pitwall agent list`)")));
    };
    // `of`: another agent's terminal to put the files next to.
    let of = match r.of.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        None => None,
        Some(o) => {
            let named: Vec<_> = views.iter().filter(|v| v.name.eq_ignore_ascii_case(o)).collect();
            let found = views.iter().find(|v| v.id == o).or((named.len() == 1).then(|| named[0]));
            Some(found.map(|v| v.id.clone()).ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent \"{o}\" in Pitwall for `of` (see `pitwall agent list`)")))?)
        }
    };
    let (mut show, skipped) = pitwall_core::show::resolve_for(&s.engine, &agent.id, &r).map_err(|e| ErrorBody::new(code::NOT_FOUND, e))?;
    show.of = of;
    let shown = backend.show(&show)?;
    let name = views.iter().find(|v| v.id == shown.agent_id).map_or_else(|| agent.name.clone(), |v| v.name.clone());
    let shown_in = match (shown.place, &shown.at) {
        (ViewerPlace::Window, _) => format!("a file viewer window of its own for {name}, showing {}", shown.root),
        (_, Some(at)) => format!("{at}, showing {}", shown.root),
        (_, None) => format!("a files tile next to {name}'s terminal (window {}), showing {}", shown.window, shown.root),
    };
    let mut opened = ViewerOpened {
        proto: VIEWER_PROTO,
        agent_id: shown.agent_id,
        agent_name: name,
        window: shown.window,
        in_background: shown.in_background,
        opened: show.files,
        folder: show.folder,
        skipped,
        place: shown.place.word().into(),
        asked: show.place.map(|p| p.word().into()),
        shown_in,
        note: shown.note,
        text: None,
    };
    opened.text = Some(mcp::text(&opened));
    Ok(opened)
}
