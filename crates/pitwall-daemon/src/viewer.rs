//! `viewer.open`: show files and folders to the user in an agent's file
//! viewer (`pitwall open`, and the `pitwall` MCP server agents get). The
//! server checks the setting, finds the agent and resolves the paths on its
//! machine (`pitwall_core::show`); the host's [`ViewerBackend`] shows them
//! in the right window. Nothing here asks the user: it only shows things,
//! read-only, in Pitwall.
//!
//! The agents' `pitwall` MCP tools are served here too (`mcp.tools`,
//! `mcp.call`): their MCP servers outlive updates, so they pass the tools
//! through instead of carrying their own. Which tools are listed follows
//! the settings (`show_web` only while agents may show web pages).
//!
//! `web.open` (and `show_web`): a web page shown to the user in the
//! agent's page tile. The address is checked here (http and https only,
//! [`pitwall_proto::web::address`]) before the host sees it.

use serde_json::Value;

use pitwall_proto::mcp::{self, McpCall, McpCalled, McpTools};
use pitwall_proto::{code, AgentFocused, ErrorBody, ViewerOpen, ViewerOpened, ViewerPlace, ViewerShow, ViewerShown, WebOpen, WebOpened, WebShow, WebShown, VIEWER_PROTO, WEB_PROTO};

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
    /// Show `req` (the address checked) for `req.agent_id`: in its page
    /// tile, else a new web tile next to its terminal, or a window.
    fn show_web(&self, req: &WebShow) -> Result<WebShown, ErrorBody> {
        let _ = req;
        Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall can't show web pages"))
    }
    /// Move the focus to agent `id`'s tile in the window that shows it,
    /// only while Pitwall is the app in front: never brings Pitwall
    /// forward. Not in front, nothing happens (`focused: false`, and why).
    fn focus_agent(&self, id: &str) -> Result<AgentFocused, ErrorBody> {
        let _ = id;
        Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall has no windows to focus an agent in"))
    }
}

/// `agent.focus`.
pub fn focus(s: &Server, r: pitwall_proto::AgentRef) -> Res {
    let Some(backend) = s.viewer.as_deref() else {
        return Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall has no windows to focus an agent in"));
    };
    let agent = crate::manage::agent(s, &r.agent_id)?;
    let o = backend.focus_agent(&agent.id)?;
    serde_json::to_value(o).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
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

/// The tools this Pitwall lists now (its settings).
fn served(s: &Server) -> McpTools {
    mcp::served(s.engine.show_files_on(), s.engine.show_web_on())
}

/// `mcp.tools`: this Pitwall's tools.
pub fn mcp_tools(s: &Server) -> Res {
    serde_json::to_value(served(s)).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// `mcp.call`: a tool call, checked against this Pitwall's tools and
/// answered as the agent sees it (a failure is a tool error that says why).
pub fn mcp_call(s: &Server, caller: &Caller, c: McpCall) -> Res {
    let failed = |e: ErrorBody| match e.code.as_str() {
        code::OFF => mcp::result(&e.message, None, true),
        _ => mcp::result(&format!("Nothing was opened: {}", e.message), None, true),
    };
    let result = match mcp::request(&c.name, &c.arguments, c.agent.clone()) {
        Err(e) => mcp::result(&format!("Nothing was opened: {e}"), None, true),
        Ok(mcp::Request::Viewer(req)) => match show(s, caller, req) {
            Ok(o) => {
                let text = o.text.clone().unwrap_or_else(|| mcp::text(&o));
                mcp::result(&text, serde_json::to_value(&o).ok(), false)
            }
            Err(e) => failed(e),
        },
        Ok(mcp::Request::Web(req)) => match web(s, caller, req) {
            Ok(o) => {
                let text = o.text.clone().unwrap_or_else(|| pitwall_proto::web::text(&o));
                mcp::result(&text, serde_json::to_value(&o).ok(), false)
            }
            Err(e) => failed(e),
        },
    };
    let McpTools { stamp, .. } = served(s);
    serde_json::to_value(McpCalled { result, stamp }).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// `web.open`'s params, read (a newer asker's word this Pitwall doesn't
/// know is said plainly, as for `viewer.open`).
pub fn web_params(p: Value) -> Result<WebOpen, ErrorBody> {
    let newer = p.get("proto").and_then(Value::as_u64).is_some_and(|v| v > u64::from(WEB_PROTO));
    serde_json::from_value(p).map_err(|e| {
        if newer {
            ErrorBody::new(code::BAD_PARAMS, format!("This Pitwall is older than its command-line tool ({e}): quit Pitwall and open it again to update it. Nothing was opened."))
        } else {
            ErrorBody::new(code::BAD_PARAMS, e.to_string())
        }
    })
}

/// `web.open`.
pub fn web_open(s: &Server, caller: &Caller, r: WebOpen) -> Res {
    let o = web(s, caller, r)?;
    serde_json::to_value(o).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// The agent `key` names (id, else a unique name), or why not.
fn agent_named(s: &Server, key: &str, what: &str) -> Result<pitwall_proto::AgentView, ErrorBody> {
    let views = s.engine.views();
    let key = key.trim();
    let named: Vec<_> = views.iter().filter(|v| v.name.eq_ignore_ascii_case(key)).collect();
    views
        .iter()
        .find(|v| v.id == key)
        .or((named.len() == 1).then(|| named[0]))
        .cloned()
        .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent \"{key}\" in Pitwall{what} (see `pitwall agent list`)")))
}

fn web(s: &Server, caller: &Caller, r: WebOpen) -> Result<WebOpened, ErrorBody> {
    let Some(backend) = s.viewer.as_deref() else {
        return Err(ErrorBody::new(code::UNSUPPORTED, "this Pitwall has no windows to show web pages in"));
    };
    if let Some(off) = s.engine.show_web_off() {
        return Err(ErrorBody::new(code::OFF, off));
    }
    // What the user could type in the address field, nothing else.
    let url = match r.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        None => None,
        Some(u) => Some(pitwall_proto::web::address(u).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e))?),
    };
    let key = r
        .agent_id
        .clone()
        .filter(|a| !a.trim().is_empty())
        .or_else(|| caller.agent.as_ref().map(|(id, _)| id.clone()))
        .ok_or_else(|| ErrorBody::new(code::BAD_PARAMS, "say whose page (--agent NAME): this doesn't run inside an agent Pitwall started"))?;
    let agent = match agent_named(s, &key, "") {
        Ok(a) => a,
        Err(e) => {
            if let Some(name) = backend.mirrored(key.trim()) {
                return Err(ErrorBody::new(code::UNSUPPORTED, format!("{name} runs on another machine and is only shown here, read-only: it can't open pages here")));
            }
            return Err(e);
        }
    };
    let of = match r.of.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        None => None,
        Some(o) => Some(agent_named(s, o, " for `of`")?.id),
    };
    let show = WebShow { agent_id: agent.id.clone(), url: url.clone(), place: r.place, of };
    let shown = backend.show_web(&show)?;
    let mut o = WebOpened {
        proto: WEB_PROTO,
        agent_id: agent.id,
        agent_name: agent.name,
        window: shown.window,
        in_background: shown.in_background,
        url,
        reused: shown.reused,
        place: shown.place.word().into(),
        asked: r.place.map(|p| p.word().into()),
        shown_in: shown.at,
        note: shown.note,
        text: None,
        ..Default::default()
    };
    o.set_browser(Some(shown.profile).filter(|p| !p.is_empty()));
    o.text = Some(pitwall_proto::web::text(&o));
    Ok(o)
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
