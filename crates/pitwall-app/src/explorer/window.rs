//! A file viewer in a window of its own: an agent showed the user files
//! with `where: window` (or no Pitwall window shows it), or the user chose
//! "Open in window" on a files tile, or dragged the tile out. A normal
//! OS window (moved, resized, minimised like any other; not kept on top
//! unless pinned), titled with the agent and the folder, with Pitwall's
//! chrome (theme, glass, the menu bar on Windows and Linux). Its place and
//! size are kept in `ui.json` (`filesWindow`) for the next one. The
//! window is the agent's: [`crate::windows::viewer_window`] finds it for
//! the agent's next call. Its grip drags it into a Pitwall window's layout
//! (`super::drag`), "Dock…" puts it in a layout slot from a menu and
//! "Dock right of the terminal" in the slot right of the agent's terminal:
//! the same viewer moves into a files tile (`super::dock`), unsaved edits
//! and all, and the window closes. ⌘P (Go to file) and ⌘⇧F (Search in
//! files) work in it as in a Pitwall window, on the folder it shows.
//! Open when Pitwall quits, it opens again at the next start ([`restore`]):
//! the same folder, tabs, open folders, place and size (`filesWindows`),
//! behind the other windows. Closed by the user, it is forgotten.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, point, prelude::*, px, size, App, Bounds, Context, Entity, Focusable, Subscription, Task,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
};

use super::quick_open::{QuickOpen, QuickOpenEvent};
use super::source::{ExplorerSource, Source};
use super::tree::TreeModel;
use super::viewer::{Host, Opened, Pane, Viewer, ViewerEvent};
use crate::agents::AgentStore;
use crate::main_screen::workspace::{DockedFiles, FilesWindow};

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
    /// The viewer's events (dropped with the viewer when it moves out),
    /// and its and its tree's changes (kept for the next start).
    viewer_sub: Option<Subscription>,
    keep_subs: Vec<Subscription>,
    /// What was last kept in `ui.json` for the next start.
    kept: Option<FilesWindow>,
    /// Its place and size now.
    bounds: [f64; 4],
    /// Its viewer moved out (the window is closing).
    moved: bool,
    handle: Option<WindowHandle<ViewerWindow>>,
    /// ⌘P's picker, open over the viewer (and what it says).
    quick: Option<(Entity<QuickOpen>, Subscription)>,
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
    /// `viewer`: one moved in (a files tile's); `None`: a new one.
    pub fn new(
        agent: String,
        key: String,
        viewer: Option<Entity<Viewer>>,
        source: Arc<dyn Source>,
        store: Entity<AgentStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let viewer = match viewer {
            Some(v) => v,
            None => Self::make_viewer(&key, &source, &store, window, cx),
        };
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
        super::viewer::guard_window_close(window, cx, move |cx| me.upgrade().filter(|w| !w.read(cx).moved).map(|w| w.read(cx).viewer.clone()));
        // Closed (by the user, or it moved into a layout, or its agent
        // went): not opened again. Pitwall quitting: it is, next start.
        cx.on_release(|this: &mut ViewerWindow, cx| {
            let agent = this.agent.clone();
            if crate::windows::quitting(cx) {
                return;
            }
            cx.defer(move |cx| {
                if !crate::windows::quitting(cx) && crate::windows::viewer_window(&agent, cx).is_none() {
                    crate::ui_state::update(cx, |ui| *ui = ui.clone().forget_files_window(&agent));
                }
            });
        })
        .detach();
        let handle = window.window_handle().downcast::<ViewerWindow>();
        let mut w = ViewerWindow {
            agent,
            key,
            viewer: viewer.clone(),
            source,
            store,
            pinned: false,
            save: None,
            viewer_sub: None,
            keep_subs: vec![],
            kept: None,
            bounds: kept_bounds(window),
            moved: false,
            handle,
            quick: None,
            _poll: poll,
            _subs: subs,
        };
        w.hold(viewer, cx);
        w.recheck_dock(cx);
        w
    }

    fn make_viewer(key: &str, source: &Arc<dyn Source>, store: &Entity<AgentStore>, window: &mut Window, cx: &mut Context<Self>) -> Entity<Viewer> {
        let ignored = crate::ui_state::get(cx).explorer_show_ignored;
        let tree = cx.new(|_| TreeModel::new(source.clone(), key.to_string(), ignored));
        tree.update(cx, |t, cx| t.refresh(false, cx));
        let (source, store, k) = (source.clone(), store.clone(), key.to_string());
        cx.new(|cx| Viewer::new(source, store, k, tree, window, cx))
    }

    /// Show `viewer` (made here or moved in) and hear its controls.
    fn hold(&mut self, viewer: Entity<Viewer>, cx: &mut Context<Self>) {
        let owner = self.store.read(cx).agent(&self.agent).map(|a| a.name.clone());
        let pinned = self.pinned;
        viewer.update(cx, |v, cx| {
            v.set_host(Host::Window, owner, cx);
            v.set_pinned(pinned, cx);
        });
        let me = cx.entity().downgrade();
        let agent = self.agent.clone();
        let tree = viewer.read(cx).tree();
        self.keep_subs = vec![cx.observe(&viewer, |this, _, cx| this.keep(cx)), cx.observe(&tree, |this, _, cx| this.keep(cx))];
        self.viewer_sub = Some(cx.subscribe(&viewer, move |this, v, e: &ViewerEvent, cx| match e {
            ViewerEvent::Pin => {
                this.pinned = !this.pinned;
                let on = this.pinned;
                let me = me.clone();
                // (The window is needed: after this update.)
                cx.defer(move |cx| {
                    if let Some(w) = me.upgrade() {
                        let h = w.read(cx).handle;
                        if let Some(h) = h {
                            let _ = h.update(cx, |_, window, _| crate::platform::keep_on_top(window, on));
                        }
                    }
                });
                v.update(cx, |v, cx| v.set_pinned(on, cx));
            }
            ViewerEvent::Place(p, of) => {
                let (p, of, agent) = (*p, of.clone(), agent.clone());
                cx.defer(move |cx| {
                    if let Err(e) = super::dock::place(&agent, p, of.as_deref(), cx) {
                        eprintln!("pitwall: could not dock the file viewer: {e}");
                    }
                });
            }
            ViewerEvent::Grip => {
                if let Some(h) = this.handle {
                    let agent = agent.clone();
                    cx.defer(move |cx| {
                        let _ = h.update(cx, |_, window, cx| super::drag::start(&agent, super::drag::From::Window(h.into()), window, cx));
                    });
                }
            }
            ViewerEvent::SetIgnored(on) => {
                let on = *on;
                v.read(cx).tree().update(cx, |t, cx| t.set_ignored(on, cx));
                crate::ui_state::update(cx, |s| s.explorer_show_ignored = on);
            }
            // "Go to file" in its empty state.
            ViewerEvent::QuickOpen => {
                if let Some(h) = this.handle {
                    cx.defer(move |cx| {
                        let _ = h.update(cx, |w, window, cx| w.quick_open(window, cx));
                    });
                }
            }
            ViewerEvent::Exit
            | ViewerEvent::Close
            | ViewerEvent::Undock
            | ViewerEvent::ShowDiff(_)
            | ViewerEvent::Opened(_) => {}
        }));
        self.viewer = viewer;
        self.keep(cx);
        cx.notify();
    }

    /// What it shows, kept in `ui.json` (when it changed) to open again
    /// after a restart.
    fn keep(&mut self, cx: &mut Context<Self>) {
        if self.moved {
            return;
        }
        let v = self.viewer.read(cx);
        let Opened { tabs, active, .. } = v.opened();
        let expanded = v.tree().read(cx).state().expanded.clone();
        let now = FilesWindow { open: DockedFiles { key: self.key.clone(), tabs, active }, expanded, bounds: Some(self.bounds) };
        if self.kept.as_ref() == Some(&now) {
            return;
        }
        self.kept = Some(now.clone());
        let agent = self.agent.clone();
        // (Not while the viewer or tree is being updated: after it.)
        cx.defer(move |cx| {
            if crate::ui_state::get(cx).files_windows.get(&agent) != Some(&now) {
                crate::ui_state::update(cx, |ui| *ui = ui.clone().keep_files_window(&agent, now));
            }
        });
    }

    /// Its viewer, moving out (into a files tile): the window closes next,
    /// without asking about unsaved edits (they go with the viewer).
    pub fn take_viewer(&mut self, cx: &mut Context<Self>) -> Entity<Viewer> {
        self.viewer_sub = None;
        self.keep_subs.clear();
        self.moved = true;
        cx.notify();
        self.viewer.clone()
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

    /// What it reads now, and its viewer.
    pub fn current(&self) -> (&str, &Entity<Viewer>) {
        (&self.key, &self.viewer)
    }

    /// Its viewer reads `key` now (its agent was removed: the folder it
    /// read, kept for unsaved edits).
    pub fn rerooted(&mut self, key: &str, cx: &mut Context<Self>) {
        self.key = key.to_string();
        self.keep(cx);
        cx.notify();
    }

    /// The viewer reading `key` (a new one, with the window's title, when
    /// it read another folder).
    pub fn viewer_for(&mut self, key: &str, title: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<Viewer> {
        if self.key != key {
            self.viewer.update(cx, |v, cx| v.hidden(cx));
            self.key = key.to_string();
            let v = Self::make_viewer(key, &self.source, &self.store, window, cx);
            self.hold(v, cx);
            self.recheck_dock(cx);
        }
        window.set_window_title(title);
        cx.notify();
        self.viewer.clone()
    }

    /// ⌘P / "Go to file": a picker over the folder it shows; the file
    /// picked opens in its viewer. Its open tabs come first, the last
    /// first.
    pub fn quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.moved {
            return;
        }
        let name = match super::source::root_of_key(&self.key) {
            Some(root) => root.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(root).to_string(),
            None => super::show::agent_name(&self.key, cx).unwrap_or_else(|| self.key.clone()),
        };
        let shown = super::show::root_display(&self.key, cx);
        let recent: Vec<String> = self.viewer.read(cx).tabs().iter().rev().cloned().collect();
        let (source, key) = (self.source.clone(), self.key.clone());
        let quick = cx.new(|cx| QuickOpen::new(source, key, name, shown, recent, window, cx));
        let sub = cx.subscribe_in(&quick, window, |this, _, e: &QuickOpenEvent, window, cx| {
            this.quick = None;
            let v = this.viewer.clone();
            match e {
                QuickOpenEvent::Picked(p) => v.update(cx, |v, cx| v.open(p.clone(), None, cx)),
                QuickOpenEvent::Dismissed => {}
            }
            window.focus(&v.focus_handle(cx), cx);
            cx.notify();
        });
        self.quick = Some((quick, sub));
        cx.notify();
    }

    /// ⌘P's picker is open (tests).
    #[cfg(test)]
    pub(crate) fn quick_open_shown(&self) -> bool {
        self.quick.is_some()
    }

    /// ⌘⇧F / "Search in files": the viewer's Search.
    fn search_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.viewer.update(cx, |v, cx| v.show(Some(Pane::Search), window, cx));
    }

    fn recheck_dock(&mut self, cx: &mut Context<Self>) {
        let can = crate::windows::showing(&self.agent, cx).is_some();
        self.viewer.update(cx, |v, cx| v.set_can_dock(can, cx));
    }

    /// Kept for the next window, half a second after the last change.
    fn bounds_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kept = kept_bounds(window);
        self.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let _ = this.update(cx, |this, cx| {
                crate::ui_state::update(cx, |s| s.files_window = Some(kept));
                this.bounds = kept;
                this.keep(cx);
            });
        }));
    }
}

/// `window`'s place and size as kept in `ui.json` (logical x, y, width,
/// height).
pub fn kept_bounds(window: &Window) -> [f64; 4] {
    let b = window.bounds();
    // (macOS reports the frame, title bar included, but opens windows
    // with that size as their content: keep the content size.)
    let content = if cfg!(target_os = "macos") { window.viewport_size() } else { b.size };
    [f64::from(f32::from(b.origin.x)), f64::from(f32::from(b.origin.y)), f64::from(f32::from(content.width)), f64::from(f32::from(content.height))]
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
            .flex_col()
            .on_action(cx.listener(|this, _: &super::GoToFile, window, cx| this.quick_open(window, cx)))
            .on_action(cx.listener(|this, _: &super::SearchInFiles, window, cx| this.search_in_files(window, cx)))
            .when(!self.moved, |d| {
                d.children(super::dock::orphan_bar(&self.agent, &self.viewer, &t, cx))
                    .child(div().flex_1().min_w_0().min_h_0().flex().bg(t.bg).child(self.viewer.clone()))
            })
            .when_some(self.quick.as_ref().map(|(q, _)| q.clone()).filter(|_| !self.moved), |d, q| {
                let over = div().absolute().inset_0();
                // Under Liquid Glass: the scrim here, the picker in a pop-up.
                d.child(if crate::kit::pop::hosted(cx) {
                    let input = q.focus_handle(cx);
                    over.child(QuickOpen::hosted(&q, input, window, cx)).into_any_element()
                } else {
                    over.child(q).into_any_element()
                })
            });
        crate::platform::decorations::frame(root, window, cx)
    }
}

/// Open a viewer window for agent `agent` reading `key`. It takes the
/// keyboard only when `focus` (Pitwall is the app in front); otherwise it
/// is ordered in without activating Pitwall.
pub fn open(agent: &str, key: &str, title: String, focus: bool, cx: &mut App) -> Result<WindowHandle<ViewerWindow>, String> {
    open_at(agent, key, title, None, None, None, focus, cx)
}

/// At start: the file viewer windows open when Pitwall quit open again,
/// behind whatever is in front, without the keyboard: each with its folder,
/// tabs (read again: unsaved edits were asked about when quitting, and are
/// not kept), open folders, place and size. One whose agent (or the agent
/// whose folder it read) is gone is forgotten, as that agent's files tile.
pub fn restore(cx: &mut App) {
    let Some(crate::ui::Content::Live { store }) = cx.try_global::<crate::AppState>().map(|s| s.content.clone()) else { return };
    let kept = crate::ui_state::get(cx).files_windows.clone();
    for (agent, k) in kept {
        let key = &k.open.key;
        let readable = super::source::root_of_key(key).is_some() || store.read(cx).agent(key).is_some();
        if store.read(cx).agent(&agent).is_none() || !readable {
            crate::ui_state::update(cx, |ui| *ui = ui.clone().forget_files_window(&agent));
            continue;
        }
        if crate::windows::viewer_window(&agent, cx).is_some() {
            continue;
        }
        let name = super::show::agent_name(&agent, cx).unwrap_or_else(|| agent.clone());
        let title = title(&name, &super::show::root_display(key, cx));
        let h = match open_at(&agent, key, title, None, None, k.bounds, false, cx) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("pitwall: could not open the file viewer window again: {e}");
                continue;
            }
        };
        let opened = Opened { tabs: k.open.tabs, active: k.open.active, pane_files: true };
        let _ = h.update(cx, |w, _, cx| {
            let v = w.viewer.clone();
            v.update(cx, |v, cx| v.restore(&opened, cx));
            let tree = v.read(cx).tree();
            tree.update(cx, |t, cx| {
                for d in &k.expanded {
                    t.expand(d, cx);
                }
            });
        });
    }
}

/// [`open`] with `viewer` moved in (a files tile's), its top left at `at`
/// on screen (a tile dragged out) or where the last window was.
pub fn open_with(agent: &str, key: &str, viewer: Option<Entity<Viewer>>, at: Option<gpui::Point<gpui::Pixels>>, focus: bool, cx: &mut App) -> Result<WindowHandle<ViewerWindow>, String> {
    let name = super::show::agent_name(agent, cx).unwrap_or_else(|| agent.to_string());
    let title = title(&name, &super::show::root_display(key, cx));
    open_at(agent, key, title, viewer, at, None, focus, cx)
}

/// `own`: the window's own place and size (one opened again at start),
/// used when it is still on a screen.
#[allow(clippy::too_many_arguments)]
fn open_at(
    agent: &str,
    key: &str,
    title: String,
    viewer: Option<Entity<Viewer>>,
    at: Option<gpui::Point<gpui::Pixels>>,
    own: Option<[f64; 4]>,
    focus: bool,
    cx: &mut App,
) -> Result<WindowHandle<ViewerWindow>, String> {
    let source = cx.try_global::<ExplorerSource>().map(|s| s.0.clone()).ok_or("no file source")?;
    let store = match &cx.try_global::<crate::AppState>().ok_or("no app state")?.content {
        crate::ui::Content::Live { store } => store.clone(),
        _ => return Err("Pitwall isn't running agents".into()),
    };
    let nth = crate::windows::viewer_windows(cx);
    let screens: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    let kept = crate::ui_state::get(cx).files_window;
    let placed = placement(own, 0, &screens).or_else(|| placement(kept, nth, &screens));
    let bounds = match (placed, at) {
        // Where a tile was let go (whole on the screen it was let go on).
        (b, Some(at)) => {
            let size = b.map_or(size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), |b| b.size);
            WindowBounds::Windowed(Bounds::new(dropped_at(at, size, &screens), size))
        }
        (Some(b), None) => WindowBounds::Windowed(b),
        (None, None) => {
            let b = Bounds::centered(None, size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)), cx);
            WindowBounds::Windowed(Bounds::new(point(b.origin.x + px(CASCADE * nth as f32), b.origin.y + px(CASCADE * nth as f32)), b.size))
        }
    };
    // Bench: shown and drawn, but off-screen, never focused (`bench.rs`),
    // as the main window.
    let (bounds, focus) = if crate::bench::enabled() && !crate::bench::visible() {
        let mut b = bounds.get_bounds();
        b.origin = point(px(crate::bench::OFF_SCREEN.0), px(crate::bench::OFF_SCREEN.1 + 900.));
        (WindowBounds::Windowed(b), false)
    } else {
        (bounds, focus)
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
            cx.new(|cx| ViewerWindow::new(a, k, viewer, source, store, window, cx))
        })
        .map_err(|e| e.to_string())?;
    crate::windows::add_viewer_window(agent, handle, cx);
    Ok(handle)
}

/// A window of `size` let go at `at`: whole on the screen holding `at`
/// (else the nearest one).
pub fn dropped_at(at: gpui::Point<gpui::Pixels>, size: gpui::Size<gpui::Pixels>, screens: &[Bounds<gpui::Pixels>]) -> gpui::Point<gpui::Pixels> {
    let dist = |b: &Bounds<gpui::Pixels>| {
        let dx = f32::from((b.left() - at.x).max(at.x - b.right()).max(px(0.)));
        let dy = f32::from((b.top() - at.y).max(at.y - b.bottom()).max(px(0.)));
        dx * dx + dy * dy
    };
    match screens.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))) {
        Some(screen) => crate::windows::fit_on(at, size, *screen),
        None => at,
    }
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
    fn a_tile_let_go_outside_opens_whole_on_that_screen() {
        let a = Bounds::new(point(px(0.), px(0.)), size(px(1600.), px(1000.)));
        let b = Bounds::new(point(px(1600.), px(0.)), size(px(1200.), px(800.)));
        let s = size(px(800.), px(600.));
        // Inside a screen: there.
        assert_eq!(dropped_at(point(px(100.), px(50.)), s, &[a, b]), point(px(100.), px(50.)));
        // Near a screen's corner: moved in to be whole.
        assert_eq!(dropped_at(point(px(1500.), px(900.)), s, &[a, b]), point(px(800.), px(400.)));
        // Off every screen: onto the nearest one.
        assert_eq!(dropped_at(point(px(3500.), px(100.)), s, &[a, b]), point(px(2000.), px(100.)));
    }

    #[test]
    fn the_title_names_the_agent_and_the_folder() {
        assert_eq!(title("api", "~/code/api"), "api — Files");
        assert_eq!(title("fix-login", "~/code/api/.worktrees/login"), "fix-login — login — Files");
        assert_eq!(title("demo", "/private/tmp/x/scratchpad/s2/demo/"), "demo — Files");
        assert_eq!(title("w", "C:\\code\\app"), "w — app — Files");
    }
}
