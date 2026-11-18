//! An agent shows the user files (`viewer.open`: `pitwall open`, the
//! `pitwall` MCP server). The server has resolved the paths on the agent's
//! machine; here they open in the agent's viewer, in the window that shows
//! that agent (`crate::windows::for_agent`), the same way a ⌘-clicked link
//! does ([`super::file_link`]). Pitwall comes forward only when it already
//! is: from another app, the viewer opens in place, unseen until the user
//! switches back.
//!
//! `where` (beside the terminal, or a window of its own) is carried along
//! but both open this viewer for now.

use std::path::{Path, PathBuf};

use gpui::{App, Context, Window};

use pitwall_proto::{code, AgentView, ErrorBody, ViewerShow, ViewerShown};

use super::file_link::{relative_to, target};
use super::viewer::Pane;
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

/// On the main thread: show `req` and say where.
pub fn show(req: &ViewerShow, cx: &mut App) -> Result<ViewerShown, ErrorBody> {
    let in_front = crate::platform::app_in_front(cx);
    let (label, handle) = crate::windows::for_agent(&req.agent_id, cx)
        .ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "Pitwall has no window open to show it in"))?;
    let shown = handle
        .update(cx, |view, window, cx| {
            let explorer = view
                .explorer()
                .ok_or_else(|| ErrorBody::new(code::UNSUPPORTED, "this Pitwall window has no file viewer"))?;
            // In front already: that window comes forward. Not: nothing is
            // brought forward (the user is in another app).
            if in_front {
                window.activate_window();
            }
            explorer.update(cx, |e, cx| e.show_request(req, window, cx))
        })
        .map_err(|_| ErrorBody::new(code::OTHER, "the Pitwall window went away"))??;
    Ok(ViewerShown { agent_id: shown, window: label, in_background: !in_front })
}

/// Agent `id`'s name when it is shown here from another machine.
pub fn mirrored(id: &str, cx: &App) -> Option<String> {
    let store = match &cx.try_global::<crate::AppState>()?.content {
        crate::ui::Content::Live { store } => store.clone(),
        _ => return None,
    };
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
    /// Show `req` in this window: its files as tabs (the first one active,
    /// at its line), its folder opened and selected in the tree. A folder
    /// outside the agent's own opens in the viewer of the agent working
    /// there. The agent whose viewer it is.
    pub fn show_request(&mut self, req: &ViewerShow, window: &mut Window, cx: &mut Context<Self>) -> Result<String, ErrorBody> {
        let agents = self.store.read(cx).agents.clone();
        let agent = agents
            .iter()
            .find(|a| a.id == req.agent_id)
            .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent \"{}\" in Pitwall", req.agent_id)))?;
        if crate::agents::mirrored(agent) {
            return Err(ErrorBody::new(code::UNSUPPORTED, format!("{} runs on another machine: Pitwall can't show its files", agent.name)));
        }
        let mut owner = agent.clone();
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
                    let Some((o, r, rel)) = there else {
                        return Err(ErrorBody::new(
                            code::UNSUPPORTED,
                            format!(
                                "{} is outside {}'s folder and no agent in Pitwall works there; the viewer shows agents' folders for now (its files can still be shown one by one)",
                                f.display(),
                                agent.name
                            ),
                        ));
                    };
                    owner = o.clone();
                    root = r;
                    folder = Some(rel);
                }
            }
        }
        if !self.can_read(&owner.id, cx) {
            return Err(ErrorBody::new(code::UNSUPPORTED, format!("{}'s files can't be read from here", owner.name)));
        }
        let files: Vec<_> = req
            .files
            .iter()
            .map(|f| {
                let p = Path::new(&f.path);
                (relative_to(&root, p).unwrap_or_else(|| f.path.clone()), target(f.line, f.column))
            })
            .collect();
        let Some(viewer) = self.viewer(&owner.id, window, cx) else {
            return Err(ErrorBody::new(code::UNSUPPORTED, format!("{}'s files can't be read from here", owner.name)));
        };
        viewer.update(cx, |v, cx| {
            for (p, at) in &files {
                v.open(p.clone(), *at, cx);
            }
            // The first one shown, at its line.
            if let Some((p, at)) = files.first().filter(|_| files.len() > 1) {
                v.open(p.clone(), *at, cx);
            }
            if let Some(dir) = &folder {
                v.reveal_folder(dir, cx);
            }
        });
        let pane = folder.is_some().then_some(Pane::Files);
        self.open_viewer(&owner.id, None, None, pane, window, cx);
        Ok(owner.id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{AppContext, TestAppContext};

    use super::*;
    use crate::agents::AgentStore;
    use crate::explorer::source::{fake::FakeSource, Source};
    use pitwall_proto::{OpenedFile, ViewerPlace};

    #[gpui::test]
    async fn files_and_folders_open_in_the_right_viewer(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("pw-ex-show-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("api/src")).unwrap();
        std::fs::create_dir_all(dir.join("api-fix/lib")).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        let path = |p: &str| root.join(p).to_string_lossy().into_owned();
        let mut a = crate::agents::tests::agent("a1", &path("api"), "idle", 0, true);
        a.caps.explorer = true;
        let mut b = crate::agents::tests::agent("a2", &path("api-fix"), "idle", 0, true);
        b.caps.explorer = true;
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
        let store = cx.new(|_| AgentStore::new(vec![a, b]));
        let source: Arc<dyn Source> = FakeSource::new(&[("src/a.rs", "a\n"), ("README.md", "# r\n"), ("lib/b.rs", "b\n")]);
        let (explorer, cx) = cx.add_window_view(move |_, cx| Explorer::with_source(Some(source), store, cx));
        let file = |p: &str, line: Option<u32>| OpenedFile { path: path(p), line, column: None };

        // Two files: both tabs, the first shown at its line.
        let req = ViewerShow {
            agent_id: "a1".into(),
            files: vec![file("api/src/a.rs", Some(4)), file("api/README.md", None)],
            folder: None,
            place: ViewerPlace::Window,
        };
        let who = cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_request(&req, window, cx))).unwrap();
        cx.run_until_parked();
        assert_eq!(who, "a1");
        let viewer = explorer.read_with(cx, |e, _| {
            assert!(e.viewer_open());
            e.viewers["a1"].clone()
        });
        viewer.read_with(cx, |v, _| {
            assert_eq!(v.tabs(), ["src/a.rs", "README.md"]);
            assert_eq!(v.active(), Some("src/a.rs"));
        });

        // A folder inside its own folder: opened in the tree.
        let req = ViewerShow { agent_id: "a1".into(), files: vec![], folder: Some(path("api/src")), place: ViewerPlace::Beside };
        cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_request(&req, window, cx))).unwrap();
        cx.run_until_parked();
        viewer.read_with(cx, |v, cx| assert!(v.tree_is_open("src", cx)));

        // Another worktree, where agent a2 works: a2's viewer.
        let req = ViewerShow { agent_id: "a1".into(), files: vec![], folder: Some(path("api-fix")), place: ViewerPlace::Beside };
        let who = cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_request(&req, window, cx))).unwrap();
        assert_eq!(who, "a2");
        explorer.read_with(cx, |e, _| assert!(e.viewers.contains_key("a2")));

        // A folder no agent works in: said so.
        let req = ViewerShow { agent_id: "a1".into(), files: vec![], folder: Some(path("elsewhere")), place: ViewerPlace::Beside };
        let e = cx.update(|window, cx| explorer.update(cx, |e, cx| e.show_request(&req, window, cx))).unwrap_err();
        assert!(e.message.contains("no agent in Pitwall works there"), "{}", e.message);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
