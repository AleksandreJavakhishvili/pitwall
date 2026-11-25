//! The read-only code explorer (docs/spec/explorer.md, inventory §14): the
//! Files tab, the file viewer, ⌘P quick open and ⇧⌘F search, over
//! `pitwall-core`'s explorer module. Strictly read-only: nothing here can
//! type into, save, create, rename or delete a file; "Copy path" is the one
//! way out.
//!
//! [`Explorer`] is one window's coordinator: one tree model per agent
//! (shared by the Files tab and the viewer), one viewer per agent (its tabs
//! and search kept for the window's session), the quick-open palette, the
//! refresh rhythm (on open, ↻ / ⌘⇧R, the agent's change totals moving, every
//! 10 s while shown). The main view mounts three things from it: the Files
//! panel ([`Explorer::panel`]), the viewer in the main area
//! ([`Explorer::main_view`]) and the Explorer itself as an overlay (quick
//! open, notices); [`on_actions`] wires the shortcuts on its root. The
//! files tiles in the layout keep their viewers in [`dock`] (one per agent
//! across windows, moved as it is between tiles and windows: [`drag`]); a
//! viewer in a window of its own is that window's ([`window`]).

pub mod code_view;
pub mod dock;
pub mod drag;
pub mod column;
pub mod file_link;
pub mod files_panel;
pub mod image_view;


pub mod logic;
pub mod pan_zoom;
pub mod pdf_view;
pub mod quick_open;
pub mod root_menu;
pub mod search;
pub mod show;
pub mod source;
pub mod tree;
pub mod tree_view;
pub mod viewer;
pub mod unsaved;
pub mod widgets;
pub mod window;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, AnyView, App, Context, Entity, EventEmitter, Focusable,
    InteractiveElement, KeyBinding, SharedString, Subscription, Task, Window,
};

use crate::agents::AgentStore;
use crate::theme::{self, RADIUS};

use files_panel::{FilesPanel, FilesPanelEvent};
use quick_open::{QuickOpen, QuickOpenEvent};
pub use source::{ExplorerSource, Source};
use tree::TreeModel;
use viewer::{Pane, Viewer, ViewerEvent};

actions!(
    explorer,
    [
        /// ⌘P: go to a file of the focused agent.
        GoToFile,
        /// ⇧⌘F: search in the focused agent's files.
        SearchInFiles,
        /// Open the file viewer on the focused agent.
        BrowseFiles,
    ]
);

/// Open folders are read again this often while shown.
pub const TREE_POLL: Duration = Duration::from_secs(10);
/// Recently opened files kept per agent (an empty ⌘P lists them first).
const RECENT: usize = 20;

/// The explorer's shortcuts, as this OS writes them (`HostInfo::shortcuts`:
/// ⌘ on macOS, Ctrl+Shift elsewhere, so Ctrl stays the terminal's).
pub fn bindings() -> Vec<KeyBinding> {
    let mut b = if cfg!(target_os = "macos") {
        vec![
            KeyBinding::new("cmd-p", GoToFile, None),
            KeyBinding::new("cmd-shift-f", SearchInFiles, None),
        ]
    } else {
        vec![
            KeyBinding::new("ctrl-shift-p", GoToFile, None),
            KeyBinding::new("ctrl-shift-alt-f", SearchInFiles, None),
        ]
    };

    b.extend(tree_view::bindings());
    b.extend(quick_open::bindings());
    b.extend(search::bindings());
    b.extend(viewer::bindings());
    b
}

/// Key bindings (call once at start).
pub fn register(cx: &mut App) {
    cx.bind_keys(bindings());
}

/// The explorer's shortcuts handled on the window's root element.
pub fn on_actions<E: InteractiveElement>(el: E, explorer: &Entity<Explorer>) -> E {
    let (a, b, c, d, f) = (
        explorer.clone(),
        explorer.clone(),
        explorer.clone(),
        explorer.clone(),
        explorer.clone(),
    );
    el.on_action(move |_: &GoToFile, window, cx| a.update(cx, |e, cx| e.go_to_file(window, cx)))
        .on_action(move |_: &SearchInFiles, window, cx| {
            b.update(cx, |e, cx| e.search_in_files(window, cx))
        })
        .on_action(move |_: &BrowseFiles, window, cx| {
            c.update(cx, |e, cx| e.browse_files(window, cx))
        })
        // ⌘-click on a file reference in a terminal.
        .on_action(move |l: &file_link::OpenFileLink, window, cx| {
            f.update(cx, |e, cx| e.open_file_link(l, window, cx))
        })
        // ⌘⇧R refreshes every source-control view: the main screen's
        // Changes and worktrees first, then the explorer here.
        .on_action(move |_: &crate::main_screen::RefreshChanges, _, cx| {
            d.update(cx, |e, cx| e.refresh_files(cx));
            cx.propagate();
        })
}

/// What the rest of the app hears from the explorer.
#[derive(Debug, Clone)]
pub enum ExplorerEvent {
    /// "Show diff": open Review on this agent and file (Review's seam).
    ShowDiff { agent_id: String, path: String },
    /// "Show diff" in a viewer reading a folder of its own (`path:`, a
    /// worktree an agent works in now): Review on that worktree and file.
    ShowWorktreeDiff { root: String, path: String },
    /// The viewer opened or closed (the right panel hides while it shows).
    ViewerToggled,
    /// Quick open (⌘P) opened: the window shows one modal at a time.
    QuickOpened,
}

pub struct Explorer {
    source: Option<Arc<dyn Source>>,
    store: Entity<AgentStore>,
    /// The focused agent.
    agent: Option<String>,
    show_ignored: bool,
    trees: HashMap<String, Entity<TreeModel>>,
    panels: HashMap<String, Entity<FilesPanel>>,
    viewers: HashMap<String, Entity<Viewer>>,
    /// The agent whose viewer is shown in the main area.
    open: Option<String>,
    quick: Option<Entity<QuickOpen>>,
    /// Quick open was asked for by this viewer (a files tile): what is
    /// picked opens there.
    quick_into: Option<gpui::WeakEntity<Viewer>>,
    /// Quick open's input (a pop-up gives it the keyboard).
    quick_input: Option<gpui::FocusHandle>,
    recents: HashMap<String, Vec<String>>,
    notice: Option<SharedString>,
    notice_task: Option<Task<()>>,
    /// Each agent's change totals when last seen (they move: read again).
    sigs: HashMap<String, (u32, u32, u32)>,
    _poll: Task<()>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ExplorerEvent> for Explorer {}

impl Explorer {
    pub fn new(store: Entity<AgentStore>, cx: &mut Context<Self>) -> Explorer {
        let source = ExplorerSource::get(cx);
        Self::with_source(source, store, cx)
    }

    pub fn with_source(
        source: Option<Arc<dyn Source>>,
        store: Entity<AgentStore>,
        cx: &mut Context<Self>,
    ) -> Explorer {
        let show_ignored = crate::ui_state::get(cx).explorer_show_ignored;
        dock::remember_env(source.clone(), &store, cx);
        let ui = crate::ui_state::store(cx);
        let subs = vec![
            cx.observe(&store, |this, _, cx| this.store_changed(cx)),
            // "Ignored" set in another window.
            cx.observe(&ui, |this, ui, cx| {
                let on = ui.read(cx).state.explorer_show_ignored;
                if on != this.show_ignored {
                    this.apply_ignored(on, cx);
                }
            }),
        ];
        let poll = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TREE_POLL).await;
            // Paused while no window shows (`document.hidden`).
            crate::platform::visible::until_any_visible(cx).await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        });
        Explorer {
            source,
            store,
            agent: None,
            show_ignored,
            trees: HashMap::new(),
            panels: HashMap::new(),
            viewers: HashMap::new(),
            open: None,
            quick: None,
            quick_into: None,
            quick_input: None,
            recents: HashMap::new(),
            notice: None,
            notice_task: None,
            sigs: HashMap::new(),
            _poll: poll,
            _subs: subs,
        }
    }

    /// The focused pane's agent changed.
    pub fn set_agent(&mut self, agent: Option<String>, cx: &mut Context<Self>) {
        if self.agent == agent {
            return;
        }
        self.agent = agent.clone();
        if let Some(id) = agent {
            if let Some(tree) = self.tree(&id, cx) {
                tree.update(cx, |t, cx| t.refresh(false, cx));
            }
        }
        cx.notify();
    }

    pub fn viewer_open(&self) -> bool {
        self.open.is_some()
    }

    /// The viewer to draw in the main area, while open.
    pub fn main_view(&self) -> Option<AnyView> {
        let id = self.open.as_ref()?;
        self.viewers.get(id).map(|v| v.clone().into())
    }

    /// The focused agent's Files tab (none when its files can't be read).
    pub fn panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyView> {
        let id = self.agent.clone()?;
        if let Some(p) = self.panels.get(&id) {
            return Some(p.clone().into());
        }
        let tree = self.tree(&id, cx)?;
        let store = self.store.clone();
        let panel = cx.new(|cx| FilesPanel::new(id.clone(), store, tree, cx));
        let agent = id.clone();
        let sub = cx.subscribe_in(
            &panel,
            window,
            move |this, _, e: &FilesPanelEvent, window, cx| match e {
                FilesPanelEvent::OpenViewer { path, search } => {
                    let pane = search.then_some(Pane::Search);
                    this.open_viewer(&agent, path.clone(), None, pane, window, cx);
                }
                FilesPanelEvent::QuickOpen => this.quick_open(&agent, window, cx),
                FilesPanelEvent::SetIgnored(on) => this.set_ignored(*on, cx),
            },
        );
        self._subs.push(sub);
        self.panels.insert(id, panel.clone());
        Some(panel.into())
    }

    fn can_read(&self, id: &str, cx: &App) -> bool {
        self.source.is_some()
            && self
                .store
                .read(cx)
                .agent(id)
                .is_some_and(|a| a.caps.explorer)
    }

    /// The agent's tree model (made on first use), if its files can be read.
    fn tree(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Entity<TreeModel>> {
        if let Some(t) = self.trees.get(id) {
            return Some(t.clone());
        }
        if !self.can_read(id, cx) {
            return None;
        }
        let source = self.source.clone()?;
        let ignored = self.show_ignored;
        let tree = cx.new(|_| TreeModel::new(source, id.to_string(), ignored));
        self.trees.insert(id.to_string(), tree.clone());
        Some(tree)
    }

    /// The agent the shortcuts act on: the viewer's while it is open, else
    /// the focused one. `None`: a notice says why.
    fn target(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let id = self.open.clone().or_else(|| self.agent.clone());
        let Some(id) = id else {
            self.notify("Focus an agent to browse its files", cx);
            return None;
        };
        if !self.can_read(&id, cx) {
            let name = self
                .store
                .read(cx)
                .agent(&id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "This agent".into());
            self.notify(format!("{name}'s files can't be read from here"), cx);
            return None;
        }
        Some(id)
    }

    pub fn go_to_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target(cx) {
            self.quick_open(&id, window, cx);
        }
    }

    /// ⌘P with `query` typed (palette entries, demos).
    pub fn go_to_file_with(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_file(window, cx);
        if let Some(q) = &self.quick {
            q.update(cx, |q, cx| q.set_query(query, cx));
        }
    }

    /// ⇧⌘F with `query` typed ("Search for the selection").
    pub fn search_for(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search_in_files(window, cx);
        if let Some(v) = self.open.as_ref().and_then(|o| self.viewers.get(o)) {
            v.update(cx, |v, cx| v.search_for(query, cx));
        }
    }

    pub fn search_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target(cx) {
            self.open_viewer(&id, None, None, Some(Pane::Search), window, cx);
        }
    }

    pub fn browse_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.target(cx) {
            self.open_viewer(&id, None, None, None, window, cx);
        }
    }

    /// ↻ / ⌘⇧R: the shown tree and file read again.
    pub fn refresh_files(&mut self, cx: &mut Context<Self>) {
        for id in self.shown_agents() {
            if let Some(t) = self.trees.get(&id) {
                t.update(cx, |t, cx| t.refresh(false, cx));
            }
            if let Some(v) = self.viewers.get(&id) {
                v.update(cx, |v, cx| v.recheck(cx));
            }
        }
    }

    fn shown_agents(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.open.iter().chain(self.agent.iter()).cloned().collect();
        ids.dedup();
        ids
    }

    /// Every 10 s: the shown trees read their open folders again, quietly
    /// (files tiles' trees: [`dock`]).
    fn poll(&mut self, cx: &mut Context<Self>) {
        for id in self.shown_agents() {
            if let Some(t) = self.trees.get(&id) {
                t.update(cx, |t, cx| t.refresh(true, cx));
            }
        }
    }

    /// The agents' change totals moved: their trees and open files are read
    /// again (quietly); agents gone drop what was kept for them.
    fn store_changed(&mut self, cx: &mut Context<Self>) {
        let agents: HashMap<String, (u32, u32, u32)> = self
            .store
            .read(cx)
            .agents
            .iter()
            .map(|a| (a.id.clone(), (a.added, a.removed, a.files_changed)))
            .collect();
        for (id, sig) in &agents {
            let before = self.sigs.insert(id.clone(), *sig);
            if before.is_some_and(|b| b != *sig) {
                if let Some(t) = self.trees.get(id) {
                    t.update(cx, |t, cx| t.refresh(true, cx));
                }
                if let Some(v) = self.viewers.get(id) {
                    v.update(cx, |v, cx| v.recheck(cx));
                }
                if let Some(v) = dock::viewer(id, cx).filter(|_| dock::key(id, cx).as_deref() == Some(id.as_str())) {
                    v.update(cx, |v, cx| v.recheck(cx));
                }
            }
        }
        let gone: Vec<String> = self
            .trees
            .keys()
            .filter(|id| !agents.contains_key(*id))
            .cloned()
            .collect();
        for id in gone {
            self.trees.remove(&id);
            self.panels.remove(&id);
            self.viewers.remove(&id);
            self.recents.remove(&id);
            self.sigs.remove(&id);
            if self.open.as_deref() == Some(id.as_str()) {
                self.open = None;
                cx.emit(ExplorerEvent::ViewerToggled);
            }
        }
        cx.notify();
    }

    /// "Show ignored files" toggled (kept in `ui.json`): every tree lists
    /// again that way.
    pub fn set_ignored(&mut self, on: bool, cx: &mut Context<Self>) {
        self.apply_ignored(on, cx);
        crate::ui_state::update(cx, |s| s.explorer_show_ignored = on);
    }

    fn apply_ignored(&mut self, on: bool, cx: &mut Context<Self>) {
        self.show_ignored = on;
        for t in self.trees.values() {
            t.update(cx, |t, cx| t.set_ignored(on, cx));
        }
        cx.notify();
    }

    /// Show the agent's viewer (on `path` at `at`, or on `pane`).
    pub fn open_viewer(
        &mut self,
        id: &str,
        path: Option<String>,
        at: Option<(u32, Option<(usize, usize)>)>,
        pane: Option<Pane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(viewer) = self.viewer(id, window, cx) else {
            return;
        };
        let was_open = self.open.as_deref() == Some(id);
        if let Some(other) = self.open.clone().filter(|o| o != id) {
            if let Some(v) = self.viewers.get(&other) {
                v.update(cx, |v, cx| v.hidden(cx));
            }
        }
        self.open = Some(id.to_string());
        viewer.update(cx, |v, cx| {
            if let Some(p) = path {
                v.open(p, at, cx);
            } else if !was_open {
                v.recheck(cx);
            }
            v.show(pane, window, cx);
        });
        if let Some(t) = self.trees.get(id) {
            t.update(cx, |t, cx| t.refresh(false, cx));
        }
        if !was_open {
            cx.emit(ExplorerEvent::ViewerToggled);
        }
        cx.notify();
    }

    /// Debug builds: agent `id`'s files tile's viewer, or (`main`) its
    /// full viewer, opened if it isn't.
    pub fn debug_viewer(&mut self, id: &str, main: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<Viewer>> {
        if !main {
            return dock::viewer(id, cx);
        }
        if self.open.as_deref() != Some(id) {
            self.open_viewer(id, None, None, None, window, cx);
        }
        self.viewers.get(id).cloned()
    }

    /// Whether one of its viewers (shown, or closed but still on screen
    /// until the next frame) has the keyboard.
    pub fn viewer_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.viewers
            .values()
            .any(|v| v.focus_handle(cx).contains_focused(window, cx))
    }

    /// Back to the agents (Esc, "Back").
    pub fn close_viewer(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.open.take() else { return };
        if let Some(v) = self.viewers.get(&id) {
            v.update(cx, |v, cx| v.hidden(cx));
        }
        cx.emit(ExplorerEvent::ViewerToggled);
        cx.notify();
    }

    fn viewer(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Viewer>> {
        if let Some(v) = self.viewers.get(id) {
            return Some(v.clone());
        }
        let tree = self.tree(id, cx)?;
        let source = self.source.clone()?;
        let store = self.store.clone();
        let agent = id.to_string();
        let viewer = cx.new(|cx| Viewer::new(source, store, agent.clone(), tree, window, cx));
        let sub = cx.subscribe_in(
            &viewer,
            window,
            move |this, _, e: &ViewerEvent, window, cx| match e {
                ViewerEvent::Exit => this.close_viewer(cx),
                ViewerEvent::QuickOpen => this.quick_open(&agent, window, cx),
                ViewerEvent::ShowDiff(path) => cx.emit(ExplorerEvent::ShowDiff {
                    agent_id: agent.clone(),
                    path: path.clone(),
                }),
                ViewerEvent::Opened(path) => this.note_recent(&agent, path),
                ViewerEvent::SetIgnored(on) => this.set_ignored(*on, cx),
                // (A tile's and a window's controls.)
                ViewerEvent::Close | ViewerEvent::Undock | ViewerEvent::Place(..) | ViewerEvent::Grip | ViewerEvent::Pin => {}
            },
        );
        self._subs.push(sub);
        self.viewers.insert(id.to_string(), viewer.clone());
        Some(viewer)
    }

    /// Quick open for files tile viewer `v` reading `key`: what is picked
    /// opens there.
    pub(crate) fn quick_open_into(&mut self, v: &Entity<Viewer>, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_into = Some(v.downgrade());
        self.quick_open(key, window, cx);
    }

    /// "Show diff" in a files tile reading `key`: Review on the agent's
    /// file, or on the worktree a `path:` key reads.
    pub(crate) fn show_diff_of(&mut self, key: &str, path: &str, cx: &mut Context<Self>) {
        cx.emit(match source::root_of_key(key) {
            None => ExplorerEvent::ShowDiff { agent_id: key.to_string(), path: path.to_string() },
            Some(root) => ExplorerEvent::ShowWorktreeDiff { root: root.to_string(), path: path.to_string() },
        });
    }

    /// A file opened in a files tile reading `key` (quick open lists it
    /// first).
    pub(crate) fn opened_in(&mut self, key: &str, path: &str) {
        self.note_recent(key, path);
    }

    fn note_recent(&mut self, id: &str, path: &str) {
        let list = self.recents.entry(id.to_string()).or_default();
        list.retain(|p| p != path);
        list.insert(0, path.to_string());
        list.truncate(RECENT);
    }

    fn quick_open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        // An agent's folder, or (a files tile) any folder here.
        let (name, shown) = match source::root_of_key(id) {
            Some(root) => (
                root.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or(root).to_string(),
                pitwall_core::paths::tildify(root),
            ),
            None => match self.store.read(cx).agent(id) {
                Some(a) => (a.name.clone(), a.cwd_display.clone()),
                None => return,
            },
        };
        let recent = self.recents.get(id).cloned().unwrap_or_default();
        let agent = id.to_string();
        let quick = cx.new(|cx| {
            QuickOpen::new(
                source,
                agent.clone(),
                name,
                shown,
                recent,
                window,
                cx,
            )
        });
        let sub = cx.subscribe_in(
            &quick,
            window,
            move |this, _, e: &QuickOpenEvent, window, cx| {
                this.quick = None;
                let into = this.quick_into.take().and_then(|v| v.upgrade());
                match e {
                    // Asked for by a files tile: it opens there.
                    QuickOpenEvent::Picked(path) if into.is_some() => {
                        if let Some(v) = into {
                            v.update(cx, |v, cx| v.open(path.clone(), None, cx));
                        }
                    }
                    QuickOpenEvent::Picked(path) => {
                        this.open_viewer(&agent, Some(path.clone()), None, None, window, cx)
                    }
                    QuickOpenEvent::Dismissed => {
                        if let Some(v) = this.open.as_ref().and_then(|o| this.viewers.get(o)) {
                            v.update(cx, |v, cx| v.show(None, window, cx));
                        }
                    }
                }
                cx.notify();
            },
        );
        sub.detach();
        self.quick_input = Some(quick.focus_handle(cx));
        self.quick = Some(quick);
        cx.emit(ExplorerEvent::QuickOpened);
        cx.notify();
    }

    /// Close quick open (another modal opened over the window).
    pub fn close_quick(&mut self, cx: &mut Context<Self>) {
        if self.quick.take().is_some() {
            cx.notify();
        }
    }

    /// A short notice at the bottom of the window (the kit's toast later).
    fn notify(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice = Some(text.into());
        self.notice_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(2600))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Demo only: a notice to look at (the bench hook's `notice <text>`).
    #[doc(hidden)]
    pub fn demo_notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.notify(text, cx);
    }

    /// Quick open shows (tests).
    #[cfg(test)]
    pub(crate) fn quick_open_shown(&self) -> bool {
        self.quick.is_some()
    }

    #[cfg(test)]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|s| s.as_ref())
    }
}

impl Render for Explorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        if let Some(q) = &self.quick {
            let over = div().absolute().inset_0();
            // Under Liquid Glass: the scrim here, the panel in a pop-up.
            return match self.quick_input.clone().filter(|_| crate::kit::pop::hosted(cx)) {
                Some(input) => over.child(QuickOpen::hosted(q, input, window, cx)),
                None => over.child(q.clone()),
            }
            .into_any_element();
        }
        // Under Liquid Glass the notice is a pop-up above the window,
        // its bottom edge's middle where the notice's is.
        if let Some(n) = self.notice.clone().filter(|_| crate::kit::pop::hosted(cx)) {
            use crate::kit::passive;
            let stamp = passive::stamp_of(&n);
            let build: passive::Build = std::rc::Rc::new(move |_, cx| {
                let t = passive::theme(cx);
                let text = div().px(px(14.)).py(px(8.)).text_size(px(12.5)).text_color(t.text_2).child(n.clone());
                passive::panel(RADIUS.into(), text).into_any_element()
            });
            let spec = passive::Passive { at: None, pin: (0.5, 1.), clicks: false, stamp, build };
            return div()
                .absolute()
                .bottom(px(28.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(passive::anchor("ex-notice", spec))
                .into_any_element();
        }
        match &self.notice {
            Some(n) => div()
                .absolute()
                .bottom(px(28.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(RADIUS)
                        .bg(t.raised)
                        .border_1()
                        .border_color(t.line_strong)
                        .shadow_md()
                        .text_size(px(12.5))
                        .text_color(t.text_2)
                        .child(n.clone()),
                )
                .into_any_element(),
            None => div().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests;

/// The `files-edit` debug command on `viewer` (in `window`): see
/// `MainView::bench`.
pub fn debug_edit(viewer: Entity<Viewer>, what: &str, window: &mut Window, cx: &mut App) {
    use gpui::EntityInputHandler as _;
    let (verb, arg) = what.split_once(' ').unwrap_or((what, ""));
    let code = viewer.read(cx).active_code();
    let focus_code = |window: &mut Window, cx: &mut App| {
        if let Some(c) = &code {
            let f = gpui::Focusable::focus_handle(c.read(cx), cx);
            window.focus(&f, cx);
        }
    };
    match verb {
        "type" => {
            focus_code(window, cx);
            if let Some(c) = &code {
                let text = arg.to_string();
                c.update(cx, |c, cx| c.replace_text_in_range(None, &text, window, cx));
            }
        }
        "keys" => {
            focus_code(window, cx);
            let keys: Vec<_> = arg.split_whitespace().filter_map(|k| gpui::Keystroke::parse_with_os(k, gpui::OperatingSystem::Mac).ok()).collect();
            window.defer(cx, move |window, cx| {
                for k in keys {
                    window.dispatch_keystroke(k, cx);
                }
            });
        }
        "goto" => {
            let mut n = arg.split_whitespace().filter_map(|v| v.parse::<usize>().ok());
            if let (Some(c), Some(line), col) = (&code, n.next(), n.next().unwrap_or(1)) {
                c.update(cx, |c, cx| c.reveal(crate::code_view::Side::New, line.max(1) - 1, Some(col.max(1) - 1..col.max(1) - 1), cx));
            }
            focus_code(window, cx);
        }
        "find" => {
            if let Some(c) = &code {
                let text = arg.to_string();
                c.update(cx, |c, cx| {
                    c.open_replace(false, window, cx);
                    c.set_find_text(&text, window, cx);
                });
            }
        }
        "replace-all" => {
            if let Some(c) = &code {
                let with = arg.to_string();
                let n = c.update(cx, |c, cx| c.replace_all(&with, cx));
                eprintln!("pitwall-debug: replaced {n}");
            }
        }
        "close-tab" => viewer.update(cx, |v, cx| {
            if let Some(p) = v.active().map(str::to_string) {
                v.request_close_tab(&p, window, cx);
            }
        }),
        "close-window" => viewer.update(cx, |v, cx| v.guard(None, window, cx, |_, window, _| window.remove_window())),
        "keep-mine" => viewer.update(cx, |v, cx| v.keep_mine(cx)),
        "take-theirs" => viewer.update(cx, |v, cx| v.reload(cx)),
        "difference" => viewer.update(cx, |v, cx| v.toggle_difference(cx)),
        "recheck" => viewer.update(cx, |v, cx| v.recheck(cx)),
        // The tree column: `tree` toggles it (as ⌘⇧E), `tree on|off`,
        // `tree-width <px>` (as dragged and let go).
        "tree" => viewer.update(cx, |v, cx| match arg {
            "on" => v.set_tree(true, window, cx),
            "off" => v.set_tree(false, window, cx),
            _ => v.toggle_tree(window, cx),
        }),
        "tree-width" => viewer.update(cx, |v, cx| {
            if let Ok(w) = arg.trim().parse::<f32>() {
                v.debug_tree_width(w, cx);
            }
        }),
        // An image's or a PDF's zoom: `zoom fit|100|in|out`.
        "zoom" => viewer.update(cx, |v, cx| v.debug_zoom(arg.trim(), cx)),
        // A PDF: `pdf find <text>|next|prev|close|page <n>`.
        "pdf" => viewer.update(cx, |v, cx| v.debug_pdf(arg.trim(), window, cx)),
        // An SVG: `svg source|picture`.
        "svg" => viewer.update(cx, |v, cx| v.set_svg_source(arg.trim() == "source", cx)),
        "open" => viewer.update(cx, |v, cx| v.open(arg.trim().to_string(), None, cx)),
        "search" => viewer.update(cx, |v, cx| v.show(Some(Pane::Search), window, cx)),
        // The window in front of Pitwall's others (to look at it).
        "front" => window.activate_window(),
        "state" => eprintln!("pitwall-debug: viewer {}", viewer.read(cx).debug_state(cx)),
        _ => eprintln!("pitwall-debug: files-edit {verb}?"),
    }
}
