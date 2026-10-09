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
//! open, notices); [`on_actions`] wires the shortcuts on its root.

pub mod code_view;
pub mod file_link;
pub mod files_panel;


pub mod logic;
pub mod quick_open;
pub mod search;
pub mod source;
pub mod tree;
pub mod tree_view;
pub mod viewer;
pub mod widgets;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, AnyView, App, Context, Entity, EventEmitter, InteractiveElement,
    KeyBinding, SharedString, Subscription, Task, Window,
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

    /// Every 10 s: the shown trees read their open folders again, quietly.
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
            },
        );
        self._subs.push(sub);
        self.viewers.insert(id.to_string(), viewer.clone());
        Some(viewer)
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
        let Some(a) = self.store.read(cx).agent(id).cloned() else {
            return;
        };
        let recent = self.recents.get(id).cloned().unwrap_or_default();
        let agent = id.to_string();
        let quick = cx.new(|cx| {
            QuickOpen::new(
                source,
                agent.clone(),
                a.name.clone(),
                a.cwd_display.clone(),
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
                match e {
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

    #[cfg(test)]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|s| s.as_ref())
    }
}

impl Render for Explorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).clone();
        if let Some(q) = &self.quick {
            return div()
                .absolute()
                .inset_0()
                .child(q.clone())
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
