//! An agent shows the user files (`viewer.open`: `pitwall open`, the
//! `pitwall` MCP server). The server has resolved the paths on the agent's
//! machine; here they open in a read-only viewer, in one of two places:
//!
//! - beside: a pane split next to the agent's terminal, in the Pitwall
//!   window that shows the agent (`crate::windows::showing`; its space is
//!   brought up in that window when another one was shown). The terminal
//!   keeps the keyboard.
//! - window: a window of its own (`super::window`), the agent's: its next
//!   call adds to it.
//!
//! The agent may name one; else beside while the user is in Pitwall and a
//! window while they are in another app (a window already open for the
//! agent is used again while in Pitwall too). Beside an agent no open
//! window shows falls back to a window, and the answer says so. Pitwall
//! comes forward only when it already is in front: from another app, the
//! window is ordered in without taking the keyboard.
//!
//! What is read: the agent's folder; a folder inside another agent's on
//! the same machine, that agent's; any other folder on this computer,
//! that folder (a `path:` key: [`super::source::path_key`]).

use std::path::{Path, PathBuf};

use gpui::{App, Context, Window};

use pitwall_proto::{code, AgentView, ErrorBody, ViewerPlace, ViewerShow, ViewerShown};

use super::file_link::{relative_to, target};
use super::source::{path_key, root_of_key};
use super::viewer::{Opened, Viewer};
use super::Explorer;

/// The setting that turns it on and off.
pub const SETTING: &str = "agents.showFiles";

/// The setting's value (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SETTING)
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

/// Where it opens. Named: there. Else, in Pitwall: beside, unless the
/// agent has a window open and nothing beside its terminal (the user moved
/// it there); in another app: a window.
pub fn placement(asked: Option<ViewerPlace>, in_front: bool, has_beside: bool, has_window: bool) -> ViewerPlace {
    if let Some(p) = asked {
        return p;
    }
    if in_front && !(has_window && !has_beside) {
        ViewerPlace::Beside
    } else {
        ViewerPlace::Window
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
    let name = agents.iter().find(|a| a.id == agent).map_or(agent.to_string(), |a| a.name.clone());
    let in_front = crate::platform::app_in_front(cx);
    let showing = crate::windows::showing(agent, cx);
    let has_beside = showing
        .as_ref()
        .and_then(|(_, h)| h.read(cx).ok())
        .and_then(|v| v.explorer())
        .is_some_and(|e| e.read(cx).beside.contains_key(agent));
    let has_window = crate::windows::viewer_window(agent, cx).is_some();
    let mut note = None;
    let root = plan.root.to_string_lossy().into_owned();
    if placement(req.place, in_front, has_beside, has_window) == ViewerPlace::Beside {
        match showing {
            Some((label, handle)) => {
                handle
                    .update(cx, |view, window, cx| {
                        let explorer = view
                            .explorer()
                            .ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "this Pitwall window has no file viewer"))?;
                        if let Some(s) = view.screen() {
                            s.update(cx, |s, cx| s.reveal_pane_of(agent, cx));
                        }
                        // In front already: that window comes forward. Not:
                        // nothing is (the user is in another app).
                        if in_front {
                            window.activate_window();
                        }
                        explorer.update(cx, |e, cx| e.show_beside(agent, &plan, window, cx))
                    })
                    .map_err(|_| ErrorBody::new(code::OTHER, "the Pitwall window went away"))??;
                return Ok(ViewerShown {
                    agent_id: agent.to_string(),
                    window: label,
                    in_background: !in_front,
                    place: ViewerPlace::Beside,
                    root,
                    note,
                });
            }
            None => note = Some(format!("{name} isn't shown in any open Pitwall window, so it opened in a window of its own.")),
        }
    }
    let title = super::window::title(&name, &root_display(&plan.key, cx));
    let handle = match crate::windows::viewer_window(agent, cx) {
        Some(h) => h,
        None => super::window::open(agent, &plan.key, title.clone(), in_front, cx).map_err(|e| ErrorBody::new(code::OTHER, e))?,
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
    Ok(ViewerShown { agent_id: agent.to_string(), window: title, in_background: !in_front, place: ViewerPlace::Window, root, note })
}

/// "Dock beside terminal": what a viewer window had open, beside agent
/// `agent`'s terminal in the window that shows it (which comes forward
/// while Pitwall is in front).
pub fn dock_beside(agent: &str, key: &str, opened: &Opened, cx: &mut App) -> Result<(), ErrorBody> {
    let (_, handle) = crate::windows::showing(agent, cx)
        .ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "no Pitwall window shows this agent"))?;
    handle
        .update(cx, |view, window, cx| {
            let explorer = view.explorer().ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "no file viewer"))?;
            if let Some(s) = view.screen() {
                s.update(cx, |s, cx| s.reveal_pane_of(agent, cx));
            }
            if crate::platform::app_in_front(cx) {
                window.activate_window();
            }
            let v = explorer
                .update(cx, |e, cx| e.beside_viewer(agent, key, window, cx))
                .ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "its files can't be read from here"))?;
            v.update(cx, |v, cx| {
                v.restore(opened, cx);
                v.show(None, window, cx);
            });
            Ok(())
        })
        .map_err(|_| ErrorBody::new(code::OTHER, "the Pitwall window went away"))?
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

impl Explorer {
    /// Show `plan` beside agent `agent`'s terminal (the pane reused, or
    /// made). The keyboard stays where it is.
    pub fn show_beside(&mut self, agent: &str, plan: &Plan, window: &mut Window, cx: &mut Context<Self>) -> Result<(), ErrorBody> {
        let viewer = self
            .beside_viewer(agent, &plan.key, window, cx)
            .ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "its files can't be read from here"))?;
        viewer.update(cx, |v, cx| apply(v, plan, cx));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{AppContext, TestAppContext};

    use super::*;
    use crate::agents::AgentStore;
    use crate::explorer::source::{fake::FakeSource, Source};
    use crate::explorer::viewer::Host;
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
        ViewerShow { agent_id: "a1".into(), files, folder, root: None, place: None }
    }

    #[test]
    fn beside_in_pitwall_a_window_from_another_app() {
        use ViewerPlace::*;
        // Not named: where the user is.
        assert_eq!(placement(None, true, false, false), Beside);
        assert_eq!(placement(None, false, false, false), Window);
        // Named: there.
        assert_eq!(placement(Some(Window), true, false, false), Window);
        assert_eq!(placement(Some(Beside), false, false, false), Beside);
        // Already in a window of its own (moved there): that one again.
        assert_eq!(placement(None, true, false, true), Window);
        assert_eq!(placement(None, true, true, true), Beside);
        // From another app the pane beside wouldn't be seen: a window.
        assert_eq!(placement(None, false, true, false), Window);
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

    fn explorer_with<'a>(
        d: &Dirs,
        cx: &'a mut TestAppContext,
    ) -> (gpui::Entity<Explorer>, &'a mut gpui::VisualTestContext) {
        let agents = d.agents();
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
        let store = cx.new(|_| AgentStore::new(agents));
        let source: Arc<dyn Source> = FakeSource::new(&[("src/a.rs", "a\n"), ("README.md", "# r\n"), ("lib/b.rs", "b\n")]);
        cx.add_window_view(move |_, cx| Explorer::with_source(Some(source), store, cx))
    }

    #[gpui::test]
    async fn the_pane_beside_is_reused_and_keeps_the_keyboard_away(cx: &mut TestAppContext) {
        let d = Dirs::new("beside");
        let agents = d.agents();
        let (explorer, cx) = explorer_with(&d, cx);
        let file = |p: &str, line| OpenedFile { path: d.at(p), line, column: None };

        let p = plan(&req(None, vec![file("api/src/a.rs", Some(4)), file("api/README.md", None)]), &agents).unwrap();
        cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_beside("a1", &p, window, cx))).unwrap();
        cx.run_until_parked();
        let viewer = explorer.read_with(cx, |e, _| {
            // Beside, not the main area.
            assert!(!e.viewer_open());
            e.beside["a1"].viewer.clone()
        });
        viewer.read_with(cx, |v, _| {
            assert_eq!(v.host(), Host::Beside);
            assert_eq!(v.tabs(), ["src/a.rs", "README.md"]);
            assert_eq!(v.active(), Some("src/a.rs"));
            assert_eq!(v.target_line(), Some(4));
        });
        // It didn't take the keyboard.
        cx.update(|window, cx| assert!(!gpui::Focusable::focus_handle(viewer.read(cx), cx).contains_focused(window, cx)));

        // The next call: the same pane, one more tab, the folder opened.
        let p = plan(&req(Some(d.at("api/src")), vec![file("api/src/a.rs", None)]), &agents).unwrap();
        cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_beside("a1", &p, window, cx))).unwrap();
        cx.run_until_parked();
        explorer.read_with(cx, |e, _| assert_eq!(e.beside["a1"].viewer, viewer));
        viewer.read_with(cx, |v, cx| {
            assert_eq!(v.tabs().len(), 2);
            assert!(v.tree_is_open("src", cx));
        });

        // A folder outside every agent's: a pane rooted there.
        let p = plan(&req(Some(d.at("elsewhere")), vec![]), &agents).unwrap();
        cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_beside("a1", &p, window, cx))).unwrap();
        cx.run_until_parked();
        let there = explorer.read_with(cx, |e, _| {
            assert_eq!(e.beside["a1"].key, path_key(&d.at("elsewhere")));
            e.beside["a1"].viewer.clone()
        });
        assert_ne!(there, viewer);
        there.read_with(cx, |v, cx| assert_eq!(v.root(cx), Some(d.at("elsewhere"))));

        // "Open in window": the pane goes, what it had open goes with it.
        let undocked = Arc::new(std::sync::Mutex::new(None));
        let got = undocked.clone();
        cx.update(|_, cx| {
            cx.subscribe(&explorer, move |_, e: &crate::explorer::ExplorerEvent, _| {
                if let crate::explorer::ExplorerEvent::Undock { agent, key, opened } = e {
                    *got.lock().unwrap() = Some((agent.clone(), key.clone(), opened.clone()));
                }
            })
            .detach();
        });
        there.update(cx, |v, cx| {
            v.open("docs/x.md".into(), None, cx);
            cx.emit(crate::explorer::viewer::ViewerEvent::Undock);
        });
        cx.run_until_parked();
        explorer.read_with(cx, |e, _| assert!(!e.beside.contains_key("a1")));
        let (agent, key, opened) = undocked.lock().unwrap().clone().unwrap();
        assert_eq!((agent.as_str(), key), ("a1", path_key(&d.at("elsewhere"))));
        assert_eq!(opened.tabs, ["docs/x.md"]);
        assert_eq!(opened.active.as_deref(), Some("docs/x.md"));
    }
}
