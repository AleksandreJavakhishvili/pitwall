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
    /// Why the viewer isn't rooted where it was asked to be.
    pub note: Option<String>,
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
    let mut note = None;
    // The viewer's folder: the root the server settled on (asked for, or
    // the worktree the agent works in now) exactly, also inside the
    // agent's own folder; else the folder asked for.
    let (key, root, rel) = match (req.root.as_deref().map(Path::new), req.folder.as_deref().map(Path::new)) {
        (None, None) => (agent.id.clone(), root_of(agent), None),
        (None, Some(f)) => viewer_of(agent, f, agents, local)?,
        (Some(r), _) => match rooted_at(agent, r, agents, local) {
            Some((key, root)) => (key, root, None),
            // Another machine: its own folder is read whole (a folder in
            // it is revealed there).
            None => {
                let (key, root, rel) = viewer_of(agent, r, agents, local)?;
                note = Some(format!(
                    "{} runs on another machine, where the viewer shows its whole folder: {} is opened in it as a folder, not as the root.",
                    agent.name,
                    r.display()
                ));
                (key, root, rel)
            }
        },
    };
    let folder = match (req.folder.as_deref().map(Path::new), req.root.is_some()) {
        (None, false) => None,
        (None, true) => rel.filter(|r| !r.is_empty()),
        (Some(_), false) => rel,
        (Some(f), true) => rel_in(&root, f),
    };
    // A folder outside the root (asked for by its full path): its own viewer.
    let (key, root, folder) = match req.folder.as_deref().map(Path::new) {
        Some(f) if folder.is_none() => viewer_of(agent, f, agents, local)?,
        _ => (key, root, folder),
    };
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
    Ok(Plan { key, root, files, folder, note })
}

/// The viewer rooted exactly at `r` for `agent`: its own (`r` is its
/// folder), another agent's on the same machine working there, else (on
/// this computer) `r` itself, also when it lies inside the agent's folder.
/// `None`: another machine's folder in it, which can't be a root of its
/// own there.
fn rooted_at(agent: &AgentView, r: &Path, agents: &[AgentView], local: bool) -> Option<(String, PathBuf)> {
    if root_of(agent) == r {
        return Some((agent.id.clone(), r.to_path_buf()));
    }
    let there = agents
        .iter()
        .filter(|o| !crate::agents::mirrored(o) && o.machine.provider == agent.machine.provider && o.machine.id == agent.machine.id && o.caps.explorer)
        .find(|o| root_of(o) == r);
    match there {
        Some(o) => Some((o.id.clone(), r.to_path_buf())),
        None if local => Some((path_key(&r.to_string_lossy()), r.to_path_buf())),
        None => None,
    }
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
    /// In the terminal's tile's place (moved there if elsewhere).
    Replace,
    /// A window of its own (the tile popped out if there is one).
    Window,
}

/// See [`Goal`].
pub fn goal(asked: Option<ViewerPlace>, tile: bool, window: bool) -> Goal {
    match asked {
        Some(ViewerPlace::Window) => Goal::Window,
        Some(ViewerPlace::Replace) => Goal::Replace,
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
    let mut note = plan.note.clone();
    let goal = goal(req.place, tile, window);
    let err = |e: String| ErrorBody::new(code::OTHER, e);
    // The terminal the files took the place of, if they did.
    let mut gone = None;
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
                    note = joined(note, format!("{whose}'s terminal isn't shown in any open Pitwall window, so the files opened in a window of their own."));
                    None
                }
            }
        }
        Goal::Replace => {
            let of = req.of.as_deref();
            match dock::spot_for(agent, FilesPlace::Replace, of, cx) {
                Ok((label, space, spot)) => {
                    gone = dock::replaced_by(&space, &spot, cx);
                    dock::dock(agent, &label, &space, &spot, Some(&plan.key), cx).map_err(err)?;
                    Some(tile_viewer(agent, &plan.key, cx).map_err(err)?)
                }
                // That terminal isn't in a layout: replaced already (by
                // these files), added to; else a window of their own.
                Err(_) if tile => Some(tile_viewer(agent, &plan.key, cx).map_err(err)?),
                Err(_) => {
                    let whose = of.map_or_else(|| name.clone(), name_of);
                    note = joined(note, format!("{whose}'s terminal isn't shown in any open Pitwall window, so the files opened in a window of their own."));
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
            at: Some(dock::with_replaced(dock::described(agent, cx), gone.as_deref(), cx)),
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
            let (key, old) = w.current();
            let carry = (key != plan.key.as_str()).then(|| replacing(old, &name, &plan.root, cx)).transpose().map_err(err)?;
            let v = w.viewer_for(&plan.key, &title, window, cx);
            v.update(cx, |v, cx| {
                if let Some(c) = &carry {
                    v.restore(c, cx);
                }
                apply(v, &plan, cx);
            });
            if in_front {
                window.activate_window();
            }
            Ok(())
        })
        .map_err(|_| ErrorBody::new(code::OTHER, "the file viewer window went away"))??;
    Ok(ViewerShown { agent_id: agent.to_string(), window: title, in_background: !in_front, place: ViewerPlace::Window, at: None, root, note })
}

/// `more` said after `note`.
fn joined(note: Option<String>, more: String) -> Option<String> {
    Some(match note {
        Some(n) => format!("{n} {more}"),
        None => more,
    })
}

/// Agent `agent`'s files tile is on side `side` of agent `anchor`'s
/// terminal already.
fn next_to(agent: &str, side: Side, anchor: &str, cx: &App) -> bool {
    dock::whereabouts_of(agent, cx) == Some(crate::main_screen::workspace::Whereabouts::Next(side, anchor.to_string()))
}

/// Agent `agent`'s tile's viewer, reading `key` (a new one in the tile when
/// it read another folder).
fn tile_viewer(agent: &str, key: &str, cx: &mut App) -> Result<Entity<Viewer>, String> {
    if let Some(v) = dock::viewer(agent, cx).filter(|_| dock::key(agent, cx).as_deref() == Some(key)) {
        return Ok(v);
    }
    let name = agent_name(agent, cx).unwrap_or_else(|| agent.to_string());
    let root = super::source::root_of_key(key).map(PathBuf::from).or_else(|| {
        live_store(cx).and_then(|s| s.read(cx).agent(key).map(root_of))
    });
    let carry = match (dock::viewer(agent, cx), root) {
        (Some(old), Some(root)) => Some(replacing(&old, &name, &root, cx)?),
        _ => None,
    };
    let (label, _, _) = dock::holder(agent, cx).ok_or("no files tile")?;
    let h = crate::windows::main_window(&label, cx).ok_or("that Pitwall window is gone")?;
    let v = h
        .update(cx, |_, window, cx| dock::make(agent, key, window, cx))
        .map_err(|_| "that Pitwall window is gone")?
        .ok_or("its files can't be read from here")?;
    if let Some(c) = &carry {
        v.update(cx, |v, cx| v.restore(c, cx));
    }
    dock::adopt(agent, key, v.clone(), cx);
    Ok(v)
}

/// A viewer showing another folder for agent `name` (another worktree,
/// say) is about to give way to one showing `root`: refused while it has
/// unsaved edits (they'd be lost). Otherwise its tabs that are there under
/// `root` too, by the same path, carry over (the same file in the other
/// worktree); the rest close.
fn replacing(old: &Entity<Viewer>, name: &str, root: &Path, cx: &App) -> Result<super::viewer::Opened, String> {
    let v = old.read(cx);
    let unsaved = v.unsaved(cx);
    if !unsaved.is_empty() {
        return Err(format!(
            "{name}'s files view shows {} with unsaved edits ({}): the user has to save or discard them before it shows {}",
            v.root(cx).map(|r| pitwall_core::paths::tildify(&r)).unwrap_or_default(),
            unsaved.join(", "),
            pitwall_core::paths::tildify(&root.to_string_lossy())
        ));
    }
    Ok(carried(&v.opened(), root))
}

/// The tabs of `o` that name a file under `root` (relative ones by the
/// same path there).
fn carried(o: &super::viewer::Opened, root: &Path) -> super::viewer::Opened {
    let there = |t: &String| {
        let p = Path::new(t);
        (if p.is_absolute() { p.to_path_buf() } else { root.join(p) }).is_file()
    };
    let tabs: Vec<String> = o.tabs.iter().filter(|t| there(t)).cloned().collect();
    let active = o.active.clone().filter(|a| tabs.contains(a));
    super::viewer::Opened { tabs, active, pane_files: o.pane_files }
}

/// Agent `id`'s name when it is shown here from another machine.
pub fn mirrored(id: &str, cx: &App) -> Option<String> {
    let store = live_store(cx)?;
    let s = store.read(cx);
    s.agents.iter().find(|a| (a.id == id || a.name.eq_ignore_ascii_case(id)) && crate::agents::mirrored(a)).map(|a| a.name.clone())
}

/// Which viewer shows folder `f` for `agent`: its own (when `f` is in its
/// folder), else the agent's working there (the deepest folder that holds
/// it), else `f` itself on this computer. With `f` relative to that root.
fn viewer_of(agent: &AgentView, f: &Path, agents: &[AgentView], local: bool) -> Result<(String, PathBuf, Option<String>), ErrorBody> {
    let root = root_of(agent);
    if let Some(rel) = rel_in(&root, f) {
        return Ok((agent.id.clone(), root, Some(rel)));
    }
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
        Some((o, r, rel)) => Ok((o.id.clone(), r, Some(rel))),
        // Any folder on this computer.
        None if local => Ok((path_key(&f.to_string_lossy()), f.to_path_buf(), Some(String::new()))),
        None => Err(ErrorBody::new(
            code::UNSUPPORTED,
            format!(
                "{} is outside {}'s folder and no agent in Pitwall works there; on another computer the viewer shows agents' folders (its files can still be shown one by one)",
                f.display(),
                agent.name
            ),
        )),
    }
}

/// `a`'s folder as the server's resolved paths spell it: links followed on
/// this computer; elsewhere as recorded.
pub(crate) fn root_of(a: &AgentView) -> PathBuf {
    if a.machine.provider == pitwall_core::provider::ProviderId::LOCAL {
        real(&a.cwd)
    } else {
        PathBuf::from(&a.cwd)
    }
}

/// `p` on this computer with links followed, spelt as the server spells
/// resolved paths (no `\\?\` on Windows); as given when it isn't there.
pub(crate) fn real(p: &str) -> PathBuf {
    use pitwall_core::exec::Exec as _;
    PathBuf::from(pitwall_core::exec::LocalExec.real_path(p).unwrap_or_else(|_| p.to_string()))
}

/// `path` relative to `root` ("" for the root itself).
fn rel_in(root: &Path, path: &Path) -> Option<String> {
    if root == path {
        return Some(String::new());
    }
    relative_to(root, path)
}

#[cfg(test)]
#[path = "show_roots_tests.rs"]
pub(crate) mod roots_tests;

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
        ViewerShow { agent_id: "a1".into(), files, folder, root: None, place: None, of: None, note: None }
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
        // In a terminal's place: there, wherever the files were.
        assert_eq!(goal(Some(Replace), true, false), Goal::Replace);
        assert_eq!(goal(Some(Replace), false, true), Goal::Replace);
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

    /// The server settled on another worktree (asked for, or where the
    /// agent works now): the viewer reads that worktree (its tree, its git
    /// state), not the folder the agent started in.
    #[test]
    fn a_root_is_the_folder_the_viewer_reads() {
        let d = Dirs::new("root");
        let agents = d.agents();
        let file = |p: &str| OpenedFile { path: d.at(p), line: None, column: None };
        let rooted = |root: &str, folder: Option<String>, files| ViewerShow { root: Some(d.at(root)), ..req(folder, files) };
        // Nobody else works in `elsewhere`: that folder, read on its own.
        let p = plan(&rooted("elsewhere", None, vec![file("elsewhere/docs/x.md")]), &agents).unwrap();
        assert_eq!((p.key, p.folder), (path_key(&d.at("elsewhere")), None));
        assert_eq!(p.files, vec![("docs/x.md".to_string(), None)]);
        // Where a2 works: a2's; a folder in it, revealed.
        let p = plan(&rooted("api-fix", Some(d.at("api-fix/lib")), vec![]), &agents).unwrap();
        assert_eq!((p.key.as_str(), p.folder.as_deref()), ("a2", Some("lib")));
        // Its own folder: its own viewer.
        let p = plan(&rooted("api", None, vec![file("api/src/a.rs")]), &agents).unwrap();
        assert_eq!((p.key.as_str(), p.files[0].0.as_str()), ("a1", "src/a.rs"));
        // A folder in it: rooted there all the same (a root is the root).
        let p = plan(&rooted("api/src", None, vec![file("api/src/a.rs")]), &agents).unwrap();
        assert_eq!((p.key, p.files[0].0.as_str(), p.note), (path_key(&d.at("api/src")), "a.rs", None));
        // Another machine's agent: its folder is read whole, the folder
        // revealed in it, and the answer says so.
        let mut far = agents.clone();
        far[0].machine.provider = "agw".into();
        let p = plan(&rooted("api/src", None, vec![]), &far).unwrap();
        assert_eq!((p.key.as_str(), p.folder.as_deref()), ("a1", Some("src")));
        assert!(p.note.unwrap_or_default().contains("another machine"));
    }

    #[test]
    fn tabs_carry_over_to_another_worktree_when_the_file_is_there() {
        let d = Dirs::new("carry");
        std::fs::write(d.root.join("api-fix/lib/x.rs"), "x").unwrap();
        let o = super::super::viewer::Opened {
            tabs: vec!["lib/x.rs".into(), "src/only-here.rs".into(), d.at("elsewhere/docs")],
            active: Some("src/only-here.rs".into()),
            pane_files: true,
        };
        let c = carried(&o, &d.root.join("api-fix"));
        assert_eq!((c.tabs, c.active), (vec!["lib/x.rs".to_string()], None));
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
