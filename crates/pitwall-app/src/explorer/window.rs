//! A file viewer in a window of its own: an agent showed the user files
//! with `where: window` (or while the user was in another app), or the
//! user chose "Open in window" on the viewer beside a terminal. A normal
//! OS window (moved, resized, minimised like any other; not kept on top
//! unless pinned), titled with the agent and the folder, with Pitwall's
//! chrome (theme, glass, the menu bar on Windows and Linux). Its place and
//! size are kept in `ui.json` (`filesWindow`) for the next one. The
//! window is the agent's: [`crate::windows::viewer_window`] finds it for
//! the agent's next call. "Dock beside terminal" moves what it has open to
//! the pane beside the agent's terminal and closes it.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, size, App, Bounds, Context, Entity, Subscription, Task,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
};

use super::source::{ExplorerSource, Source};
use super::tree::TreeModel;
use super::viewer::{Host, Opened, Viewer, ViewerEvent};
use crate::agents::AgentStore;

/// A new window's size when none was kept.
const DEFAULT_SIZE: (f32, f32) = (900., 680.);
const MIN_SIZE: (f32, f32) = (420., 300.);
/// Each further window is set off from the last by this much.
const CASCADE: f32 = 26.;

pub struct ViewerWindow {
    /// The agent it was opened for.
    agent: String,
    /// What it reads (that agent's id, another's, or a `path:` key).
    key: String,
    viewer: Entity<Viewer>,
    source: Arc<dyn Source>,
    store: Entity<AgentStore>,
    pinned: bool,
    save: Option<Task<()>>,
    _poll: Task<()>,
    _subs: Vec<Subscription>,
}

/// What a window title says: the agent and the folder's name (the whole
/// folder is in the viewer's bar, shortened, with the full path on hover;
/// the title, also in the Window menu and Mission Control, stays short).
pub fn title(agent_name: &str, root_display: &str) -> String {
    let folder = root_display.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default();
    if folder.is_empty() || folder == agent_name {
        format!("{agent_name} — Files")
    } else {
        format!("{agent_name} — {folder} — Files")
    }
}

/// Where a new window goes: the kept place (when it is still on a screen)
/// set off by `nth` steps, else centred on the main screen.
pub fn placement(kept: Option<[f64; 4]>, nth: usize, screens: &[Bounds<gpui::Pixels>]) -> Option<Bounds<gpui::Pixels>> {
    let [x, y, w, h] = kept?;
    if w < f64::from(MIN_SIZE.0) || h < f64::from(MIN_SIZE.1) {
        return None;
    }
    let step = CASCADE * nth as f32;
    let b = Bounds::new(point(px(x as f32 + step), px(y as f32 + step)), size(px(w as f32), px(h as f32)));
    // Some of it (its title bar's corner) on a screen there is now.
    let corner = point(b.origin.x + px(40.), b.origin.y + px(10.));
    screens.iter().any(|s| s.contains(&corner)).then_some(b)
}

impl ViewerWindow {
    pub fn new(agent: String, key: String, source: Arc<dyn Source>, store: Entity<AgentStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let viewer = Self::make_viewer(&agent, &key, &source, &store, window, cx);
        let ui = crate::ui_state::store(cx);
        let subs = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                crate::theme::os_changed(cx);
                crate::theme::apply_window(window, cx);
            }),
            crate::settings::attach_window(window, cx),
            cx.observe_window_bounds(window, |this, window, cx| this.bounds_changed(window, cx)),
            // A space moved, a window opened or closed: can it dock?
            cx.observe(&ui, |this, _, cx| this.recheck_dock(cx)),
            cx.observe(&store, |this, _, cx| this.recheck_dock(cx)),
        ];
        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(super::TREE_POLL).await;
            crate::platform::visible::until_any_visible(cx).await;
            let ok = this.update(cx, |this, cx| {
                let tree = this.viewer.read(cx).tree();
                tree.update(cx, |t, cx| t.refresh(true, cx));
            });
            if ok.is_err() {
                break;
            }
        });
        // Closing it with unsaved changes asks first.
        let me = cx.entity().downgrade();
        super::viewer::guard_window_close(window, cx, move |cx| me.upgrade().map(|w| w.read(cx).viewer.clone()));
        let mut w = ViewerWindow { agent, key, viewer, source, store, pinned: false, save: None, _poll: poll, _subs: subs };
        w.recheck_dock(cx);
        w
    }

    fn make_viewer(
        agent: &str,
        key: &str,
        source: &Arc<dyn Source>,
        store: &Entity<AgentStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Viewer> {
        let ignored = crate::ui_state::get(cx).explorer_show_ignored;
        let tree = cx.new(|_| TreeModel::new(source.clone(), key.to_string(), ignored));
        tree.update(cx, |t, cx| t.refresh(false, cx));
        let owner = store.read(cx).agent(agent).map(|a| a.name.clone());
        let (source, store, k) = (source.clone(), store.clone(), key.to_string());
        let viewer = cx.new(|cx| {
            let mut v = Viewer::new(source, store, k, tree, window, cx);
            v.set_host(Host::Window, owner, cx);
            v
        });
        cx.subscribe_in(&viewer, window, |this, v, e: &ViewerEvent, window, cx| match e {
            ViewerEvent::Pin => {
                this.pinned = !this.pinned;
                crate::platform::keep_on_top(window, this.pinned);
                let on = this.pinned;
                v.update(cx, |v, cx| v.set_pinned(on, cx));
            }
            ViewerEvent::Dock => this.dock(window, cx),
            ViewerEvent::SetIgnored(on) => {
                let on = *on;
                v.read(cx).tree().update(cx, |t, cx| t.set_ignored(on, cx));
                crate::ui_state::update(cx, |s| s.explorer_show_ignored = on);
            }
            ViewerEvent::Exit
            | ViewerEvent::Close
            | ViewerEvent::Undock
            | ViewerEvent::QuickOpen
            | ViewerEvent::ShowDiff(_)
            | ViewerEvent::Opened(_) => {}
        })
        .detach();
        viewer
    }

    pub fn agent(&self) -> &str {
        &self.agent
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn viewer(&self) -> &Entity<Viewer> {
        &self.viewer
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// The viewer reading `key` (a new one, with the window's title, when
    /// it read another folder).
    pub fn viewer_for(&mut self, key: &str, title: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<Viewer> {
        if self.key != key {
            self.viewer.update(cx, |v, cx| v.hidden(cx));
            self.key = key.to_string();
            self.viewer = Self::make_viewer(&self.agent, key, &self.source, &self.store, window, cx);
            let on = self.pinned;
            self.viewer.update(cx, |v, cx| v.set_pinned(on, cx));
            self.recheck_dock(cx);
        }
        window.set_window_title(title);
        cx.notify();
        self.viewer.clone()
    }

    fn recheck_dock(&mut self, cx: &mut Context<Self>) {
        let can = crate::windows::showing(&self.agent, cx).is_some();
        self.viewer.update(cx, |v, cx| v.set_can_dock(can, cx));
    }

    /// "Dock beside terminal": what it has open goes beside the agent's
    /// terminal, and the window closes.
    fn dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (agent, key, opened) = (self.agent.clone(), self.key.clone(), self.viewer.read(cx).opened());
        let me = window.window_handle();
        cx.defer(move |cx| {
            match super::show::dock_beside(&agent, &key, &opened, cx) {
                Ok(()) => {
                    let _ = me.update(cx, |_, window, _| window.remove_window());
                }
                Err(e) => eprintln!("pitwall: could not dock the file viewer: {}", e.message),
            }
        });
    }

    /// Kept for the next window, half a second after the last change.
    fn bounds_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let b = window.bounds();
        // (macOS reports the frame, title bar included, but opens windows
        // with that size as their content: keep the content size.)
        let content = if cfg!(target_os = "macos") { window.viewport_size() } else { b.size };
        let kept = [f64::from(f32::from(b.origin.x)), f64::from(f32::from(b.origin.y)), f64::from(f32::from(content.width)), f64::from(f32::from(content.height))];
        self.save = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            cx.update(|cx| crate::ui_state::update(cx, |s| s.files_window = Some(kept)));
        }));
    }
}

impl Render for ViewerWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = crate::theme::theme(cx).clone();
        let root = div()
            .id("viewer-window")
            .relative()
            .size_full()
            .flex()
            .bg(crate::theme::backdrop(cx))
            .font_family(crate::kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(crate::kit::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(div().flex_1().min_w_0().min_h_0().flex().bg(t.bg).child(self.viewer.clone()));
        crate::platform::decorations::frame(root, window, cx)
    }
}

/// Open a viewer window for agent `agent` reading `key`. It takes the
/// keyboard only when `focus` (Pitwall is the app in front); otherwise it
/// is ordered in without activating Pitwall.
pub fn open(agent: &str, key: &str, title: String, focus: bool, cx: &mut App) -> Result<WindowHandle<ViewerWindow>, String> {
    let source = cx.try_global::<ExplorerSource>().map(|s| s.0.clone()).ok_or("no file source")?;
    let store = match &cx.try_global::<crate::AppState>().ok_or("no app state")?.content {
        crate::ui::Content::Live { store } => store.clone(),
        _ => return Err("Pitwall isn't running agents".into()),
    };
    let nth = crate::windows::viewer_windows(cx);
    let screens: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    let kept = crate::ui_state::get(cx).files_window;
    let bounds = match placement(kept, nth, &screens) {
        Some(b) => WindowBounds::Windowed(b),
        None => {
            let b = Bounds::centered(None, size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), cx);
            WindowBounds::Windowed(Bounds::new(point(b.origin.x + px(CASCADE * nth as f32), b.origin.y + px(CASCADE * nth as f32)), b.size))
        }
    };
    let options = WindowOptions {
        window_bounds: Some(bounds),
        titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
        focus,
        show: true,
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        window_decorations: crate::platform::decorations::request(),
        app_id: Some("dev.pitwall.app".into()),
        ..Default::default()
    };
    let (a, k) = (agent.to_string(), key.to_string());
    let handle = cx
        .open_window(options, move |window, cx| {
            crate::platform::lean_renderer(window);
            cx.new(|cx| ViewerWindow::new(a, k, source, store, window, cx))
        })
        .map_err(|e| e.to_string())?;
    crate::windows::add_viewer_window(agent, handle, cx);
    Ok(handle)
}

/// Move what the pane beside agent `agent`'s terminal had open into a
/// window of its own ("Open in window"), which comes forward while Pitwall
/// is in front.
pub fn undock(agent: &str, key: &str, opened: &Opened, cx: &mut App) -> Result<(), String> {
    let name = crate::explorer::show::agent_name(agent, cx).unwrap_or_else(|| agent.to_string());
    let root = super::show::root_display(key, cx);
    let title = title(&name, &root);
    let handle = match crate::windows::viewer_window(agent, cx) {
        Some(h) => h,
        None => {
            let front = crate::platform::app_in_front(cx);
            open(agent, key, title.clone(), front, cx)?
        }
    };
    handle
        .update(cx, |w, window, cx| {
            let v = w.viewer_for(key, &title, window, cx);
            v.update(cx, |v, cx| v.restore(opened, cx));
            // (The user clicked in Pitwall: it is in front, and the window
            // comes forward among its windows. Never over another app.)
            if crate::platform::app_in_front(cx) {
                window.activate_window();
            }
            v.update(cx, |v, cx| v.show(None, window, cx));
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_window_opens_where_the_last_one_was_left() {
        let screen = Bounds::new(point(px(0.), px(0.)), size(px(1600.), px(1000.)));
        let kept = Some([100., 120., 800., 600.]);
        let b = placement(kept, 0, &[screen]).unwrap();
        assert_eq!((b.origin.x, b.origin.y, b.size.width, b.size.height), (px(100.), px(120.), px(800.), px(600.)));
        // A second one at the same time: set off from it.
        let b = placement(kept, 1, &[screen]).unwrap();
        assert_eq!((b.origin.x, b.origin.y), (px(100. + CASCADE), px(120. + CASCADE)));
        // Left on a screen that is gone, or too small: placed afresh.
        assert_eq!(placement(Some([3000., 100., 800., 600.]), 0, &[screen]), None);
        assert_eq!(placement(Some([100., 100., 80., 60.]), 0, &[screen]), None);
        assert_eq!(placement(None, 0, &[screen]), None);
    }

    #[test]
    fn the_title_names_the_agent_and_the_folder() {
        assert_eq!(title("api", "~/code/api"), "api — Files");
        assert_eq!(title("fix-login", "~/code/api/.worktrees/login"), "fix-login — login — Files");
        assert_eq!(title("demo", "/private/tmp/x/scratchpad/s2/demo/"), "demo — Files");
        assert_eq!(title("w", "C:\\code\\app"), "w — app — Files");
    }
}
