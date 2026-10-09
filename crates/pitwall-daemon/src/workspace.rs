//! `space.*` and `agent.move`: Pitwall's spaces and windows over the socket
//! (`pitwall space`). They live in the host's UI state (the app's
//! `ui.json` store), so the host applies them through a
//! [`WorkspaceBackend`], live in every window. Nothing here asks the user:
//! it only rearranges Pitwall's own windows.

use serde_json::Value;

use pitwall_proto::{code, AgentMove, ErrorBody, SpaceCreate, SpaceMove, SpaceRename, SpaceView};

use crate::server::Server;

/// The host's spaces. Called on connection threads; may block while the
/// host's UI thread applies the change.
pub trait WorkspaceBackend: Send + Sync {
    fn spaces(&self) -> Result<Vec<SpaceView>, ErrorBody>;
    /// A new custom space named `name`, in the main window.
    fn create(&self, name: &str) -> Result<SpaceView, ErrorBody>;
    /// `space`: an id or a name ([`find`]).
    fn rename(&self, space: &str, name: &str) -> Result<SpaceView, ErrorBody>;
    /// Into `window`: `new`, `main` or an open window's label.
    fn move_to_window(&self, space: &str, window: &str) -> Result<SpaceView, ErrorBody>;
    /// Show `agent` (known to exist) in `space`, as dropping it on its tab.
    fn move_agent(&self, agent: &str, space: &str) -> Result<SpaceView, ErrorBody>;
}

type Res = Result<Value, ErrorBody>;

fn ok(v: impl serde::Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// A space by id, else by name (ignoring case; it must be the only one).
pub fn find<'a>(spaces: &'a [SpaceView], key: &str) -> Result<&'a SpaceView, ErrorBody> {
    let key = key.trim();
    if let Some(s) = spaces.iter().find(|s| s.id == key) {
        return Ok(s);
    }
    let named: Vec<&SpaceView> = spaces.iter().filter(|s| s.name.eq_ignore_ascii_case(key)).collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => Err(ErrorBody::new(code::NOT_FOUND, format!("no space \"{key}\" (see `pitwall space list`)"))),
        more => Err(ErrorBody::new(
            code::CONFLICT,
            format!("{} spaces are named \"{key}\": use an id ({})", more.len(), more.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")),
        )),
    }
}

fn backend(s: &Server) -> Result<&dyn WorkspaceBackend, ErrorBody> {
    s.workspace.as_deref().ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "this Pitwall has no windows to arrange"))
}

pub fn list(s: &Server) -> Res {
    ok(backend(s)?.spaces()?)
}

pub fn create(s: &Server, r: SpaceCreate) -> Res {
    let name = r.name.trim();
    if name.is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "a space needs a name"));
    }
    ok(backend(s)?.create(name)?)
}

pub fn rename(s: &Server, r: SpaceRename) -> Res {
    if r.name.trim().is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "a space needs a name"));
    }
    ok(backend(s)?.rename(&r.space, r.name.trim())?)
}

pub fn move_to_window(s: &Server, r: SpaceMove) -> Res {
    if r.window.trim().is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "which window: new, main or a window's label"));
    }
    ok(backend(s)?.move_to_window(&r.space, r.window.trim())?)
}

pub fn move_agent(s: &Server, r: AgentMove) -> Res {
    let b = backend(s)?;
    crate::manage::agent(s, &r.agent_id)?;
    ok(b.move_agent(&r.agent_id, &r.space)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(id: &str, name: &str) -> SpaceView {
        SpaceView { id: id.into(), name: name.into(), kind: "custom".into(), project: None, window: "main".into(), shown: vec![], members: vec![] }
    }

    #[test]
    fn spaces_are_found_by_id_then_unique_name() {
        let all = [space("all", "All"), space("s1", "API"), space("s2", "web"), space("s3", "Web")];
        assert_eq!(find(&all, "s1").unwrap().id, "s1");
        assert_eq!(find(&all, "api").unwrap().id, "s1");
        assert_eq!(find(&all, " All ").unwrap().id, "all");
        assert_eq!(find(&all, "web").unwrap_err().code, code::CONFLICT);
        assert_eq!(find(&all, "nope").unwrap_err().code, code::NOT_FOUND);
    }
}
