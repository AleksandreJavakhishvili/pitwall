//! An agent shows the user files (`viewer.open`: `pitwall open`, the
//! `pitwall` MCP server). The server has resolved the paths on the agent's
//! machine; here they open in the agent's file viewer, in one of two
//! places:
//!
//! - a files tile in the layout of the Pitwall window that shows the
//!   agent (`crate::windows::showing`), a peer of the terminal tiles: right
//!   (the default), left, above or below the agent's terminal, or of the
//!   terminal `of` names (`super::dock`). Its space is brought up in that
//!   window when another one was shown. The terminal keeps the keyboard.
//! - window: a window of its own (`super::window`).
//!
//! One viewer per agent: a later call adds tabs to it wherever it is (a
//! tile the user moved, a window they popped it out to); a place named
//! that isn't where it is moves it there, as the same viewer (its unsaved
//! edits too). A tile for an agent no open window shows falls back to a
//! window, and the answer says so. Pitwall comes forward only when it
//! already is in front: from another app, a window is ordered in without
//! taking the keyboard.
//!
//! What is read: the agent's folder; a folder inside another agent's on
//! the same machine, that agent's; any other folder on this computer,
//! that folder (a `path:` key: [`super::source::path_key`]).

use std::path::{Path, PathBuf};

use gpui::{App, Context, Entity};

use pitwall_proto::{code, AgentView, ErrorBody, ViewerPlace, ViewerShow, ViewerShown};

use super::file_link::{relative_to, target};
use super::source::{path_key, root_of_key};
use super::dock::{self, FilesPlace};
use super::viewer::Viewer;
use crate::main_screen::tree::Side;

/// The setting that turns it on and off.
pub const SETTING: &str = "agents.showFiles";

/// The setting's value (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// The setting that lets the user edit files in the viewer.
pub const EDIT_SETTING: &str = "agents.editFiles";

/// Editing in the viewer is on (unless turned off).
pub fn edit_enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(EDIT_SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// A line (1-based) and the columns on it to mark.
pub type At = (u32, Option<(usize, usize)>);

/// What to open, and in which viewer.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// What the viewer reads: an agent's id or a folder's `path:` key.
    pub key: String,
    /// That folder.
    pub root: PathBuf,
    /// Tabs (relative to `root`, else absolute) and where to scroll.
    pub files: Vec<(String, Option<At>)>,
    /// The folder to open in the tree ("" for the root).
    pub folder: Option<String>,
}

/// `req`'s viewer: see the module. `agents` are Pitwall's.
pub fn plan(req: &ViewerShow, agents: &[AgentView]) -> Result<Plan, ErrorBody> {
    let agent = agents
        .iter()
        .find(|a| a.id == req.agent_id)
        .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent \"{}\" in Pitwall", req.agent_id)))?;
    if crate::agents::mirrored(agent) {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("{} runs on another machine: Pitwall can't show its files", agent.name)));
    }
    let local = agent.machine.provider == pitwall_core::provider::ProviderId::LOCAL;
    let mut key = agent.id.clone();
    let mut root = root_of(agent);
    let mut folder = None;
    if let Some(f) = req.folder.as_deref().map(Path::new) {
        match rel_in(&root, f) {
            Some(rel) => folder = Some(rel),
            None => {
                // The agent working there (the deepest folder that holds it).
                let there = agents
                    .iter()
                    .filter(|o| !crate::agents::mirrored(o) && o.machine.provider == agent.machine.provider && o.machine.id == agent.machine.id)
                    .filter(|o| o.caps.explorer)
                    .filter_map(|o| {
                        let r = root_of(o);
                        rel_in(&r, f).map(|rel| (o, r, rel))
                    })
                    .max_by_key(|(_, r, _)| r.as_os_str().len());
                match there {
                    Some((o, r, rel)) => {
                        key = o.id.clone();
                        root = r;
                        folder = Some(rel);
                    }
                    // Any folder on this computer.
                    None if local => {
                        key = path_key(&f.to_string_lossy());
                        root = f.to_path_buf();
                        folder = Some(String::new());
                    }
                    None => {
                        return Err(ErrorBody::new(
                            code::UNSUPPORTED,
                            format!(
                                "{} is outside {}'s folder and no agent in Pitwall works there; on another computer the viewer shows agents' folders (its files can still be shown one by one)",
                                f.display(),
                                agent.name
                            ),
                        ))
                    }
                }
            }
        }
    }
    if root_of_key(&key).is_none() && !agents.iter().any(|a| a.id == key && a.caps.explorer) {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("{}'s files can't be read from here", agent.name)));
    }
    let files = req
        .files
        .iter()
        .map(|f| {
            let p = Path::new(&f.path);
            (relative_to(&root, p).unwrap_or_else(|| f.path.clone()), target(f.line, f.column))
        })
        .collect();
    Ok(Plan { key, root, files, folder })
}

/// Where agent `agent`'s files go for `asked`: a window, or a tile on
/// that side of a terminal. `None`: where they are (`tile` a tile, `window`
/// a window), else a tile right of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// Add to the tile where it is.
    Tile,
    /// Add to the window where it is.
    InWindow,
    /// A tile on that side of the terminal (moved there if elsewhere).
    Next(Side),
    /// A window of its own (the tile popped out if there is one).
    Window,
}

/// See [`Goal`].
pub fn goal(asked: Option<ViewerPlace>, tile: bool, window: bool) -> Goal {
    match asked {
        Some(ViewerPlace::Window) => Goal::Window,
        Some(p) => Goal::Next(side_of(p)),
        None if tile => Goal::Tile,
        None if window => Goal::InWindow,
        None => Goal::Next(Side::Right),
    }
}

fn side_of(p: ViewerPlace) -> Side {
    match p {
        ViewerPlace::Left => Side::Left,
        ViewerPlace::Above => Side::Top,
        ViewerPlace::Below => Side::Bottom,
        _ => Side::Right,
    }
}

/// The tabs, line and folder of `plan` in `v`.
fn apply(v: &mut Viewer, plan: &Plan, cx: &mut Context<Viewer>) {
    for (p, at) in &plan.files {
        v.open(p.clone(), *at, cx);
    }
    // The first one shown, at its line.
    if let Some((p, at)) = plan.files.first().filter(|_| plan.files.len() > 1) {
        v.open(p.clone(), *at, cx);
    }
    if let Some(dir) = &plan.folder {
        v.reveal_folder(dir, cx);
    }
}

fn live_store(cx: &App) -> Option<gpui::Entity<crate::agents::AgentStore>> {
    match &cx.try_global::<crate::AppState>()?.content {
        crate::ui::Content::Live { store } => Some(store.clone()),
        _ => None,
    }
}

/// Agent `id`'s name.
pub fn agent_name(id: &str, cx: &App) -> Option<String> {
    live_store(cx)?.read(cx).agent(id).map(|a| a.name.clone())
}

/// The folder `key` reads, for people.
pub fn root_display(key: &str, cx: &App) -> String {
    match root_of_key(key) {
        Some(r) => pitwall_core::paths::tildify(r),
        None => live_store(cx)
            .and_then(|s| s.read(cx).agent(key).map(|a| a.cwd_display.clone()))
            .unwrap_or_default(),
    }
}

/// On the main thread: show `req` and say where.
pub fn show(req: &ViewerShow, cx: &mut App) -> Result<ViewerShown, ErrorBody> {
    let store = live_store(cx).ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "Pitwall isn't running agents"))?;
    let agents = store.read(cx).agents.clone();
    let plan = plan(req, &agents)?;
    let agent = req.agent_id.as_str();
    let name_of = |id: &str| agents.iter().find(|a| a.id == id).map_or(id.to_string(), |a| a.name.clone());
    let name = name_of(agent);
    let in_front = crate::platform::app_in_front(cx);
    let tile = dock::holder(agent, cx).is_some() && dock::viewer(agent, cx).is_some();
    let window = crate::windows::viewer_window(agent, cx).is_some();
    let root = plan.root.to_string_lossy().into_owned();
    let mut note = None;
    let goal = goal(req.place, tile, window);
    let err = |e: String| ErrorBody::new(code::OTHER, e);
    let tiled = match goal {
        Goal::Tile => Some(tile_viewer(agent, &plan.key, cx).map_err(err)?),
        Goal::Next(side) => {
            let of = req.of.as_deref().filter(|o| *o != agent);
            match dock::spot_for(agent, FilesPlace::Next(side), of, cx) {
                Ok((label, space, spot)) => {
                    // Where it is already: added to, not moved.
                    let there = tile && next_to(agent, side, of.unwrap_or(agent), cx);
                    if !there {
                        dock::dock(agent, &label, &space, &spot, Some(&plan.key), cx).map_err(err)?;
                    }
                    Some(tile_viewer(agent, &plan.key, cx).map_err(err)?)
                }
                Err(_) => {
                    let whose = of.map_or_else(|| name.clone(), name_of);
                    note = Some(format!("{whose}'s terminal isn't shown in any open Pitwall window, so the files opened in a window of their own."));
                    None
                }
            }
        }
        Goal::InWindow | Goal::Window => None,
    };
    if let Some(v) = tiled {
        v.update(cx, |v, cx| apply(v, &plan, cx));
        let (label, _, _) = dock::holder(agent, cx).unwrap_or_default();
        // In front already: that window comes forward. Not: nothing does
        // (the user is in another app). The keyboard stays where it is.
        if in_front {
            if let Some(h) = crate::windows::main_window(&label, cx) {
                let _ = h.update(cx, |_, window, _| window.activate_window());
            }
        }
        return Ok(ViewerShown {
            agent_id: agent.to_string(),
            window: label,
            in_background: !in_front,
            place: req.place.filter(|p| p.is_tile()).unwrap_or_default(),
            at: Some(dock::described(agent, cx)),
            root,
            note,
        });
    }
    let title = super::window::title(&name, &root_display(&plan.key, cx));
    let handle = match crate::windows::viewer_window(agent, cx) {
        Some(h) => h,
        // The tile's viewer (popped out), else a new one.
        None if dock::viewer(agent, cx).is_some() => dock::pop_out(agent, None, cx).map_err(err)?,
        None => super::window::open(agent, &plan.key, title.clone(), in_front, cx).map_err(err)?,
    };
    handle
        .update(cx, |w, window, cx| {
            let v = w.viewer_for(&plan.key, &title, window, cx);
            v.update(cx, |v, cx| apply(v, &plan, cx));
            if in_front {
                window.activate_window();
            }
        })
        .map_err(|_| ErrorBody::new(code::OTHER, "the file viewer window went away"))?;
    Ok(ViewerShown { agent_id: agent.to_string(), window: title, in_background: !in_front, place: ViewerPlace::Window, at: None, root, note })
}

/// Agent `agent`'s files tile is on side `side` of agent `anchor`'s
/// terminal already.
fn next_to(agent: &str, side: Side, anchor: &str, cx: &App) -> bool {
    let ui = crate::ui_state::get(cx);
    let Some((space, pane)) = ui.files_tile(agent) else { return false };
    ui.space(&space).and_then(|sp| dock::neighbour(&sp.layout, &pane)) == Some((side, anchor.to_string()))
}

/// Agent `agent`'s tile's viewer, reading `key` (a new one in the tile when
/// it read another folder).
fn tile_viewer(agent: &str, key: &str, cx: &mut App) -> Result<Entity<Viewer>, String> {
    if let Some(v) = dock::viewer(agent, cx).filter(|_| dock::key(agent, cx).as_deref() == Some(key)) {
        return Ok(v);
    }
    let (label, _, _) = dock::holder(agent, cx).ok_or("no files tile")?;
    let h = crate::windows::main_window(&label, cx).ok_or("that Pitwall window is gone")?;
    let v = h
        .update(cx, |_, window, cx| dock::make(agent, key, window, cx))
        .map_err(|_| "that Pitwall window is gone")?
        .ok_or("its files can't be read from here")?;
    dock::adopt(agent, key, v.clone(), cx);
    Ok(v)
}

/// Agent `id`'s name when it is shown here from another machine.
pub fn mirrored(id: &str, cx: &App) -> Option<String> {
    let store = live_store(cx)?;
    let s = store.read(cx);
    s.agents.iter().find(|a| (a.id == id || a.name.eq_ignore_ascii_case(id)) && crate::agents::mirrored(a)).map(|a| a.name.clone())
}

/// `a`'s folder as the server's resolved paths spell it: links followed on
/// this computer; elsewhere as recorded.
fn root_of(a: &AgentView) -> PathBuf {
    let cwd = PathBuf::from(&a.cwd);
    if a.machine.provider == pitwall_core::provider::ProviderId::LOCAL {
        std::fs::canonicalize(&cwd).unwrap_or(cwd)
    } else {
        cwd
    }
}

/// `path` relative to `root` ("" for the root itself).
fn rel_in(root: &Path, path: &Path) -> Option<String> {
    if root == path {
        return Some(String::new());
    }
    relative_to(root, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::OpenedFile;

    struct Dirs {
        dir: PathBuf,
        root: PathBuf,
    }

    impl Dirs {
        fn new(tag: &str) -> Dirs {
            let dir = std::env::temp_dir().join(format!("pw-ex-show-{tag}-{}", std::process::id()));
            for d in ["api/src", "api-fix/lib", "elsewhere/docs"] {
                std::fs::create_dir_all(dir.join(d)).unwrap();
            }
            let root = std::fs::canonicalize(&dir).unwrap();
            Dirs { dir, root }
        }
        fn at(&self, p: &str) -> String {
            self.root.join(p).to_string_lossy().into_owned()
        }
        fn agents(&self) -> Vec<AgentView> {
            let mut a = crate::agents::tests::agent("a1", &self.at("api"), "idle", 0, true);
            a.caps.explorer = true;
            let mut b = crate::agents::tests::agent("a2", &self.at("api-fix"), "idle", 0, true);
            b.caps.explorer = true;
            vec![a, b]
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn req(folder: Option<String>, files: Vec<OpenedFile>) -> ViewerShow {
        ViewerShow { agent_id: "a1".into(), files, folder, root: None, place: None, of: None }
    }

    #[test]
    fn a_tile_right_of_the_terminal_unless_named_or_already_open() {
        use ViewerPlace::*;
        // Nothing open, nothing named: a tile right of the terminal.
        assert_eq!(goal(None, false, false), Goal::Next(Side::Right));
        // Open already: added to where it is (a tile, a window).
        assert_eq!(goal(None, true, false), Goal::Tile);
        assert_eq!(goal(None, false, true), Goal::InWindow);
        // Named: there (moved there if it was elsewhere).
        assert_eq!(goal(Some(Window), true, false), Goal::Window);
        assert_eq!(goal(Some(Above), false, true), Goal::Next(Side::Top));
        assert_eq!(goal(Some(Below), true, false), Goal::Next(Side::Bottom));
        assert_eq!(goal(Some(Left), false, false), Goal::Next(Side::Left));
        assert_eq!(goal(Some(Right), true, true), Goal::Next(Side::Right));
    }

    #[test]
    fn folders_go_to_the_agent_working_there_or_open_where_they_are() {
        let d = Dirs::new("plan");
        let agents = d.agents();
        let file = |p: &str, line| OpenedFile { path: d.at(p), line, column: None };
        // Its own files: relative tabs, the line kept.
        let p = plan(&req(None, vec![file("api/src/a.rs", Some(4)), file("elsewhere/x.md", None)]), &agents).unwrap();
        assert_eq!(p.key, "a1");
        assert_eq!(p.files, vec![("src/a.rs".to_string(), Some((4, None))), (d.at("elsewhere/x.md"), None)]);
        // A folder inside its own.
        let p = plan(&req(Some(d.at("api/src")), vec![]), &agents).unwrap();
        assert_eq!((p.key.as_str(), p.folder.as_deref()), ("a1", Some("src")));
        // In another worktree, where a2 works: a2's.
        let p = plan(&req(Some(d.at("api-fix/lib")), vec![]), &agents).unwrap();
        assert_eq!((p.key.as_str(), p.folder.as_deref()), ("a2", Some("lib")));
        // Anywhere else on this computer: that folder, as the root.
        let p = plan(&req(Some(d.at("elsewhere")), vec![file("elsewhere/docs/x.md", Some(2))]), &agents).unwrap();
        assert_eq!(p.key, path_key(&d.at("elsewhere")));
        assert_eq!(p.folder.as_deref(), Some(""));
        assert_eq!(p.files, vec![("docs/x.md".to_string(), Some((2, None)))]);
        // Another machine's agent: refused.
        let mut far = agents.clone();
        far[0].machine.link = Some(pitwall_proto::peer::LinkView {
            peer: "studio".into(),
            state: pitwall_proto::peer::LinkState::Connected,
            since: 0,
            last_seen: None,
        });
        assert!(crate::agents::mirrored(&far[0]));
        let e = plan(&req(Some(d.at("elsewhere")), vec![]), &far).unwrap_err();
        assert!(e.message.contains("another machine"), "{}", e.message);
    }

    /// A later call adds to the agent's files tile: the same viewer, one
    /// more tab, the folder opened; it never takes the keyboard.
    #[gpui::test]
    async fn later_calls_add_to_the_same_files_tile(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let d = Dirs::new("tile");
        let agents = d.agents();
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
        let store = cx.new(|_| crate::agents::AgentStore::new(agents.clone()));
        let source: std::sync::Arc<dyn crate::explorer::source::Source> =
            crate::explorer::source::fake::FakeSource::new(&[("src/a.rs", "a\n"), ("README.md", "# r\n")]);
        cx.update(|cx| dock::remember_env(Some(source), &store, cx));
        let (_, cx) = cx.add_window_view(|_, _| gpui::Empty);
        let v = cx.update(|window, cx| dock::make("a1", "a1", window, cx)).unwrap();
        cx.update(|_, cx| dock::adopt("a1", "a1", v.clone(), cx));
        let file = |p: &str, line| OpenedFile { path: d.at(p), line, column: None };

        let p = plan(&req(None, vec![file("api/src/a.rs", Some(4)), file("api/README.md", None)]), &agents).unwrap();
        let got = cx.update(|_, cx| tile_viewer("a1", &p.key, cx)).unwrap();
        assert_eq!(got, v);
        got.update(cx, |v, cx| apply(v, &p, cx));
        cx.run_until_parked();
        v.read_with(cx, |v, _| {
            assert_eq!(v.tabs(), ["src/a.rs", "README.md"]);
            assert_eq!(v.active(), Some("src/a.rs"));
            assert_eq!(v.target_line(), Some(4));
        });
        cx.update(|window, cx| assert!(!gpui::Focusable::focus_handle(v.read(cx), cx).contains_focused(window, cx)));

        let p = plan(&req(Some(d.at("api/src")), vec![file("api/src/a.rs", None)]), &agents).unwrap();
        let got = cx.update(|_, cx| tile_viewer("a1", &p.key, cx)).unwrap();
        assert_eq!(got, v);
        got.update(cx, |v, cx| apply(v, &p, cx));
        cx.run_until_parked();
        v.read_with(cx, |v, cx| {
            assert_eq!(v.tabs().len(), 2);
            assert!(v.tree_is_open("src", cx));
        });
    }
}
