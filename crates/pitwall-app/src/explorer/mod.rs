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
//! screen asks it for the viewers agents opened beside their terminals
//! ([`Explorer::beside_view`], [`show`]); a viewer in a window of its own
//! is that window's ([`window`]).

pub mod code_view;
pub mod column;
pub mod file_link;
pub mod files_panel;
pub mod image_view;


pub mod logic;
pub mod quick_open;
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
use viewer::{Host, Pane, Viewer, ViewerEvent};

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
    /// A viewer beside a terminal opened, closed or changed folder (the
    /// screen draws the panes again).
    BesideChanged,
    /// "Open in window" on the viewer beside agent `agent`'s terminal:
    /// it moves to a window of its own, with what it had open.
    Undock { agent: String, key: String, opened: viewer::Opened },
}

/// A viewer an agent opened beside its terminal.
pub struct Beside {
    pub viewer: Entity<Viewer>,
    /// What it reads (the agent's id or a folder's `path:` key).
    pub key: String,
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
    /// Viewers beside agents' terminals, by agent.
    pub(crate) beside: HashMap<String, Beside>,
    /// Trees of folders that aren't an agent's (`path:` keys).
    path_trees: HashMap<String, Entity<TreeModel>>,
    /// The agent whose viewer is shown in the main area.
    open: Option<String>,
    quick: Option<Entity<QuickOpen>>,
    /// Quick open was asked for by this viewer (beside a terminal): what
    /// is picked opens there.
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
            beside: HashMap::new(),
            path_trees: HashMap::new(),
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
    /// (the viewers beside terminals' too).
    fn poll(&mut self, cx: &mut Context<Self>) {
        for id in self.shown_agents() {
            if let Some(t) = self.trees.get(&id) {
                t.update(cx, |t, cx| t.refresh(true, cx));
            }
        }
        for t in self.beside_trees() {
            t.update(cx, |t, cx| t.refresh(true, cx));
        }
    }

    fn beside_trees(&self) -> Vec<Entity<TreeModel>> {
        let mut out: Vec<Entity<TreeModel>> = vec![];
        for b in self.beside.values() {
            if let Some(t) = self.trees.get(&b.key).or_else(|| self.path_trees.get(&b.key)) {
                if !out.iter().any(|o| o == t) {
                    out.push(t.clone());
                }
            }
        }
        out
    }

    /// The viewer beside agent `agent`'s terminal, if one is open.
    pub fn beside_view(&self, agent: &str) -> Option<AnyView> {
        self.beside.get(agent).map(|b| b.viewer.clone().into())
    }

    /// Close the viewer beside `agent`'s terminal (the keyboard goes back
    /// to the terminal when it was in it).
    pub fn close_beside(&mut self, agent: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.beside.remove(agent) else { return };
        if b.viewer.focus_handle(cx).contains_focused(window, cx) {
            window.blur(cx);
        }
        b.viewer.update(cx, |v, cx| v.hidden(cx));
        self.drop_unused_path_trees();
        cx.emit(ExplorerEvent::BesideChanged);
        cx.notify();
    }

    fn drop_unused_path_trees(&mut self) {
        let used: Vec<String> = self.beside.values().map(|b| b.key.clone()).collect();
        self.path_trees.retain(|k, _| used.contains(k));
    }

    /// The tree for `key`: an agent's (shared with its Files tab), or a
    /// folder's (made on first use).
    fn tree_for(&mut self, key: &str, cx: &mut Context<Self>) -> Option<Entity<TreeModel>> {
        if source::root_of_key(key).is_none() {
            return self.tree(key, cx);
        }
        if let Some(t) = self.path_trees.get(key) {
            return Some(t.clone());
        }
        let source = self.source.clone()?;
        let ignored = self.show_ignored;
        let tree = cx.new(|_| TreeModel::new(source, key.to_string(), ignored));
        self.path_trees.insert(key.to_string(), tree.clone());
        Some(tree)
    }

    /// The viewer beside `agent`'s terminal reading `key`: the one open
    /// (a new one when it read another folder), made when there's none.
    /// It doesn't take the keyboard: the terminal keeps it.
    pub fn beside_viewer(
        &mut self,
        agent: &str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Viewer>> {
        if let Some(b) = self.beside.get(agent).filter(|b| b.key == key) {
            return Some(b.viewer.clone());
        }
        let tree = self.tree_for(key, cx)?;
        let source = self.source.clone()?;
        let store = self.store.clone();
        let owner = self.store.read(cx).agent(agent).map(|a| a.name.clone());
        let k = key.to_string();
        let viewer = cx.new(|cx| {
            let mut v = Viewer::new(source, store, k, tree, window, cx);
            v.set_host(Host::Beside, owner, cx);
            v
        });
        let (a, k) = (agent.to_string(), key.to_string());
        let sub = cx.subscribe_in(&viewer, window, move |this, v, e: &ViewerEvent, window, cx| match e {
            // Esc: the keyboard back to the terminal.
            ViewerEvent::Exit => {
                if v.focus_handle(cx).contains_focused(window, cx) {
                    window.blur(cx);
                }
            }
            ViewerEvent::Close => this.close_beside(&a, window, cx),
            ViewerEvent::Undock => {
                let opened = v.read(cx).opened();
                this.close_beside(&a, window, cx);
                cx.emit(ExplorerEvent::Undock { agent: a.clone(), key: k.clone(), opened });
            }
            ViewerEvent::QuickOpen => {
                this.quick_into = Some(v.downgrade());
                this.quick_open(&k, window, cx);
            }
            ViewerEvent::ShowDiff(path) => {
                if source::root_of_key(&k).is_none() {
                    cx.emit(ExplorerEvent::ShowDiff { agent_id: k.clone(), path: path.clone() })
                }
            }
            ViewerEvent::Opened(path) => this.note_recent(&k, path),
            ViewerEvent::SetIgnored(on) => this.set_ignored(*on, cx),
            ViewerEvent::Dock | ViewerEvent::Pin => {}
        });
        self._subs.push(sub);
        if let Some(old) = self.beside.insert(agent.to_string(), Beside { viewer: viewer.clone(), key: key.to_string() }) {
            old.viewer.update(cx, |v, cx| v.hidden(cx));
        }
        self.drop_unused_path_trees();
        if let Some(t) = self.trees.get(key).or_else(|| self.path_trees.get(key)) {
            t.update(cx, |t, cx| t.refresh(false, cx));
        }
        cx.emit(ExplorerEvent::BesideChanged);
        cx.notify();
        Some(viewer)
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
                for b in self.beside.values().filter(|b| b.key == *id) {
                    b.viewer.update(cx, |v, cx| v.recheck(cx));
                }
            }
        }
        // An agent gone: the viewer beside its terminal goes with it.
        let gone_beside: Vec<String> = self.beside.keys().filter(|id| !agents.contains_key(*id)).cloned().collect();
        for id in gone_beside {
            if let Some(b) = self.beside.remove(&id) {
                b.viewer.update(cx, |v, cx| v.hidden(cx));
            }
            cx.emit(ExplorerEvent::BesideChanged);
        }
        self.drop_unused_path_trees();
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
        for t in self.trees.values().chain(self.path_trees.values()) {
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

    /// Debug builds: agent `id`'s viewer beside its terminal, or (`main`)
    /// its full viewer, opened if it isn't.
    pub fn debug_viewer(&mut self, id: &str, main: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<Viewer>> {
        if !main {
            return self.beside.get(id).map(|b| b.viewer.clone());
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
                // (A pane's and a window's controls.)
                ViewerEvent::Close | ViewerEvent::Undock | ViewerEvent::Dock | ViewerEvent::Pin => {}
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
        // An agent's folder, or (beside a terminal) any folder here.
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
                    // Asked for beside a terminal: it opens there.
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
        // An image's zoom: `zoom fit|100|in|out`.
        "zoom" => viewer.update(cx, |v, cx| v.debug_zoom(arg.trim(), cx)),
        "open" => viewer.update(cx, |v, cx| v.open(arg.trim().to_string(), None, cx)),
        "search" => viewer.update(cx, |v, cx| v.show(Some(Pane::Search), window, cx)),
        // The window in front of Pitwall's others (to look at it).
        "front" => window.activate_window(),
        "state" => eprintln!("pitwall-debug: viewer {}", viewer.read(cx).debug_state(cx)),
        _ => eprintln!("pitwall-debug: files-edit {verb}?"),
    }
}
