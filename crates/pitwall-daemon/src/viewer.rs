//! `viewer.open`: show files and folders to the user in an agent's file
//! viewer (`pitwall open`, and the `pitwall` MCP server agents get). The
//! server checks the setting, finds the agent and resolves the paths on its
//! machine (`pitwall_core::show`); the host's [`ViewerBackend`] shows them
//! in the right window. Nothing here asks the user: it only shows things,
//! read-only, in Pitwall.

use serde_json::Value;

use pitwall_proto::{code, ErrorBody, ViewerOpen, ViewerOpened, ViewerShow, ViewerShown};

use crate::identity::Caller;
use crate::server::Server;

/// The host's windows. Called on connection threads; may block while the
/// host's UI thread opens the viewer.
pub trait ViewerBackend: Send + Sync {
    /// Open `req` (paths already resolved and checked) in the viewer of
    /// `req.agent_id` (or of the agent working in `req.folder`), in the
    /// window that shows that agent.
    fn show(&self, req: &ViewerShow) -> Result<ViewerShown, ErrorBody>;
    /// The name of agent `id` when it is one shown here from another
    /// machine (a peer's), which has no files here.
    fn mirrored(&self, id: &str) -> Option<String>;
}

type Res = Result<Value, ErrorBody>;

/// What to say when `agents.showFiles` is off.
pub const OFF: &str = "Showing files is turned off in Pitwall (Settings → Agents → “Agents can show you files”, or `pitwall settings set agents.showFiles true`). Nothing was opened.";

pub fn open(s: &Server, caller: &Caller, r: ViewerOpen) -> Res {
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
    let (show, skipped) = pitwall_core::show::resolve_for(&s.engine, &agent.id, &r).map_err(|e| ErrorBody::new(code::NOT_FOUND, e))?;
    let shown = backend.show(&show)?;
    let name = views.iter().find(|v| v.id == shown.agent_id).map_or_else(|| agent.name.clone(), |v| v.name.clone());
    let opened = ViewerOpened {
        agent_id: shown.agent_id,
        agent_name: name.clone(),
        window: shown.window,
        in_background: shown.in_background,
        opened: show.files,
        folder: show.folder,
        skipped,
        place: show.place,
        shown_in: format!("{name}'s file viewer"),
    };
    serde_json::to_value(opened).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}
